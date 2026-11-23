// #10036 S7 (design #9539 §6): attachment storage refs, multipart
// shaping, and the record's attachments field.
import { describe, expect, test } from "bun:test";
import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createHash } from "node:crypto";
import {
  MAX_ATTACHMENTS_PER_PROMPT,
  MAX_ATTACHMENT_BYTES,
  PromptAttachmentsStore,
  isPromptAttachmentRef,
  normalizeAttachments,
  readMultipartForm,
  sanitizeAttachmentName,
  submissionFieldsFromForm,
  uploadedFilesFromForm,
} from "./prompt-attachments.ts";
import { PromptsStore, promptAnswerBody, validateNewPrompt, type NewPrompt } from "./prompts.ts";
import { renderPromptWidget } from "./prompt-widget.ts";
import { loadConfig } from "./config.ts";

const text = (s: string) => new TextEncoder().encode(s);

describe("PromptAttachmentsStore", () => {
  test("saves a blob and reads it back by ref with matching metadata", async () => {
    const dir = await mkdtemp(join(tmpdir(), "prompt-att-"));
    try {
      const store = new PromptAttachmentsStore(join(dir, "attachments"));
      const bytes = text("hello attachment");
      const entry = await store.save({ name: "note.txt", contentType: "text/plain", bytes });
      expect(isPromptAttachmentRef(entry.ref)).toBe(true);
      expect(entry.name).toBe("note.txt");
      expect(entry.size).toBe(bytes.byteLength);

      const found = await store.read(entry.ref);
      expect(found).not.toBeNull();
      expect(new TextDecoder().decode(found!.bytes)).toBe("hello attachment");
      expect(found!.meta.sha256).toBe(createHash("sha256").update(bytes).digest("hex"));
      // The blob on disk is named by the uuid only — the ref is abstract.
      const uuid = entry.ref.slice("attachment:".length);
      expect(await readFile(join(dir, "attachments", uuid))).toEqual(Buffer.from(bytes));
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("refuses forged and traversal-shaped refs without touching disk", async () => {
    const dir = await mkdtemp(join(tmpdir(), "prompt-att-"));
    try {
      const store = new PromptAttachmentsStore(join(dir, "attachments"));
      expect(await store.read("attachment:../prompts")).toBeNull();
      expect(await store.read("../../etc/passwd")).toBeNull();
      expect(await store.read("attachment:not-a-uuid")).toBeNull();
      expect(await store.read("attachment:123e4567-e89b-12d3-a456-426614174000")).toBeNull();
      await store.remove("attachment:../prompts"); // must not throw
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("remove deletes the blob and its sidecar", async () => {
    const dir = await mkdtemp(join(tmpdir(), "prompt-att-"));
    try {
      const store = new PromptAttachmentsStore(join(dir, "attachments"));
      const entry = await store.save({ name: "a.bin", contentType: "", bytes: text("x") });
      expect(entry.contentType).toBe("application/octet-stream");
      await store.remove(entry.ref);
      expect(await store.read(entry.ref)).toBeNull();
      expect(await readdir(join(dir, "attachments"))).toEqual([]);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("saveAll enforces the count and per-file limits before writing anything", async () => {
    const dir = await mkdtemp(join(tmpdir(), "prompt-att-"));
    try {
      const store = new PromptAttachmentsStore(join(dir, "attachments"));
      const many = Array.from({ length: MAX_ATTACHMENTS_PER_PROMPT + 1 }, (_, i) => ({
        name: `f${i}.txt`,
        contentType: "text/plain",
        bytes: text("x"),
      }));
      await expect(store.saveAll(many)).rejects.toThrow(/too many attachments/);
      const oversized = [
        { name: "big.bin", contentType: "application/octet-stream", bytes: new Uint8Array(MAX_ATTACHMENT_BYTES + 1) },
      ];
      await expect(store.saveAll(oversized)).rejects.toThrow(/exceeds/);
      // Nothing was written for either refused upload.
      await expect(readdir(join(dir, "attachments"))).rejects.toThrow();
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("sanitizes display names so they cannot carry a path", () => {
    expect(sanitizeAttachmentName("../../etc/passwd")).toBe(".._.._etc_passwd");
    expect(sanitizeAttachmentName('a"b\n.txt')).toBe("ab.txt");
    expect(sanitizeAttachmentName("")).toBe("attachment");
    expect(sanitizeAttachmentName("report final.pdf")).toBe("report final.pdf");
  });
});

describe("normalizeAttachments", () => {
  test("keeps well-formed entries and drops the rest", () => {
    const good = {
      ref: "attachment:123e4567-e89b-12d3-a456-426614174000",
      name: "x.png",
      contentType: "image/png",
      size: 42,
    };
    const out = normalizeAttachments([
      good,
      { ref: "attachment:../evil", name: "e", contentType: "text/plain", size: 1 },
      { ref: good.ref, name: "y", contentType: "text/plain", size: -1 },
      "nope",
      null,
    ]);
    expect(out).toEqual([good]);
    expect(normalizeAttachments(undefined)).toEqual([]);
    expect(normalizeAttachments("nope")).toEqual([]);
  });
});

describe("multipart shaping", () => {
  function formWith(fields: Record<string, string>): FormData {
    const form = new FormData();
    for (const [k, v] of Object.entries(fields)) form.set(k, v);
    return form;
  }

  test("shapes text fields into the validator's JSON body, coercing notify and refs", () => {
    const form = formWith({
      scope: "page",
      target: "claude",
      mode: "async",
      page: "queue",
      prompt: "look at this",
      notify: "true",
      refs: '["prompt:9"]',
    });
    const shaped = submissionFieldsFromForm(form);
    expect(shaped).toEqual({
      scope: "page",
      target: "claude",
      mode: "async",
      page: "queue",
      prompt: "look at this",
      notify: true,
      refs: ["prompt:9"],
    });
    expect(validateNewPrompt(shaped).ok).toBe(true);
  });

  test("rejects a refs field that is not a JSON array", () => {
    const form = formWith({ refs: "not json" });
    expect(() => submissionFieldsFromForm(form)).toThrow(/refs must be a JSON array/);
  });

  test("extracts file parts and skips the untouched file input's empty part", async () => {
    const form = new FormData();
    form.set("prompt", "hi");
    form.append("attachments", new File([text("one")], "one.txt", { type: "text/plain" }));
    form.append("attachments", new File([], "", { type: "application/octet-stream" }));
    form.append("attachments", new File([text("two")], "two.png", { type: "image/png" }));
    const files = await uploadedFilesFromForm(form);
    expect(files.map((f) => f.name)).toEqual(["one.txt", "two.png"]);
    expect(files[1].contentType).toBe("image/png");
    expect(new TextDecoder().decode(files[0].bytes)).toBe("one");
  });

  test("readMultipartForm parses a real multipart request end to end", async () => {
    const form = new FormData();
    form.set("scope", "page");
    form.set("prompt", "with file");
    form.append("attachments", new File([text("payload")], "p.txt", { type: "text/plain" }));
    const req = new Request("http://aggregator.test/api/prompt", { method: "POST", body: form });
    const parsed = await readMultipartForm(req);
    expect(parsed.get("prompt")).toBe("with file");
    const files = await uploadedFilesFromForm(parsed);
    expect(files.length).toBe(1);
    expect(new TextDecoder().decode(files[0].bytes)).toBe("payload");
  });
});

describe("prompt record attachments (S7)", () => {
  const valid: NewPrompt = {
    scope: "page",
    target: "claude",
    mode: "async",
    page: "queue",
    sectionRef: null,
    prompt: "see attached",
  };

  test("a submission's refs persist on the record and survive a reload", async () => {
    const dir = await mkdtemp(join(tmpdir(), "prompts-s7-"));
    try {
      const path = join(dir, "prompts.json");
      const attachments = [
        { ref: "attachment:123e4567-e89b-12d3-a456-426614174000", name: "x.png", contentType: "image/png", size: 42 },
      ];
      const store = new PromptsStore(path);
      const record = await store.add({ ...valid, attachments });
      expect(record.attachments).toEqual(attachments);
      const onDisk = JSON.parse(await readFile(path, "utf8"));
      expect(onDisk.prompts[0].attachments).toEqual(attachments);

      const reloaded = new PromptsStore(path);
      expect((await reloaded.get(record.id))?.attachments).toEqual(attachments);
      // The §3.7 answer body echoes the refs.
      const answered = { ...record, status: "answered" as const, answer: "done" };
      expect(promptAnswerBody(answered).attachments).toEqual(attachments);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("records written before S7 load with an empty attachments array", async () => {
    const dir = await mkdtemp(join(tmpdir(), "prompts-s7-"));
    try {
      const path = join(dir, "prompts.json");
      const legacy = {
        version: 1,
        prompts: [
          {
            scope: "page",
            target: "claude",
            mode: "async",
            page: "queue",
            sectionRef: null,
            prompt: "old",
            id: "prompt:old",
            threadId: "prompt:old",
            status: "queued",
            createdAt: "2026-11-20T00:00:00.000Z",
            archived: false,
            read: false,
            replyTo: null,
            refs: [],
            notify: false,
          },
        ],
      };
      await Bun.write(path, JSON.stringify(legacy));
      const store = new PromptsStore(path);
      expect((await store.get("prompt:old"))?.attachments).toEqual([]);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});

describe("S7 wiring", () => {
  test("the widget offers a file input on a multipart form", () => {
    const html = renderPromptWidget({ scope: "page", page: "queue" });
    expect(html).toContain('enctype="multipart/form-data"');
    expect(html).toContain('<input type="file" name="attachments" multiple');
  });

  test("the attachments dir defaults next to the prompts file and honors its env override", () => {
    const cfg = loadConfig({ PM_AGG_DEV: "1", PM_AGG_PROMPTS_FILE: "/data/stack/prompts.json" });
    expect(cfg.promptAttachmentsDir).toBe("/data/stack/prompt-attachments");
    const overridden = loadConfig({
      PM_AGG_DEV: "1",
      PM_AGG_PROMPTS_FILE: "/data/stack/prompts.json",
      PM_AGG_PROMPT_ATTACHMENTS_DIR: "/mnt/blobs",
    });
    expect(overridden.promptAttachmentsDir).toBe("/mnt/blobs");
  });
});
