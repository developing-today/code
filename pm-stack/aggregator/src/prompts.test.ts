import { describe, expect, test } from "bun:test";
import { mkdtemp, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PromptsStore, validateNewPrompt, type NewPrompt } from "./prompts.ts";
import { renderPromptWidget } from "./prompt-widget.ts";

const valid: NewPrompt = {
  scope: "page",
  target: "claude",
  mode: "async",
  page: "queue",
  sectionRef: null,
  prompt: "reprioritize",
};

describe("validateNewPrompt", () => {
  test("accepts a well-formed submission", () => {
    const r = validateNewPrompt(valid);
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.value).toEqual(valid);
  });

  test("rejects unknown scope, target and mode", () => {
    expect(validateNewPrompt({ ...valid, scope: "global" }).ok).toBe(false);
    expect(validateNewPrompt({ ...valid, target: "pool:low" }).ok).toBe(false);
    expect(validateNewPrompt({ ...valid, mode: "sync-block" }).ok).toBe(false);
  });

  test("requires sectionRef for section scope", () => {
    expect(validateNewPrompt({ ...valid, scope: "section" }).ok).toBe(false);
    expect(validateNewPrompt({ ...valid, scope: "section", sectionRef: "proj-1" }).ok).toBe(true);
  });

  test("rejects empty, oversized and non-object prompts", () => {
    expect(validateNewPrompt({ ...valid, prompt: "   " }).ok).toBe(false);
    expect(validateNewPrompt({ ...valid, prompt: "x".repeat(8001) }).ok).toBe(false);
    expect(validateNewPrompt(null).ok).toBe(false);
    expect(validateNewPrompt([]).ok).toBe(false);
  });

  test("rejects page ids that are not slugs", () => {
    expect(validateNewPrompt({ ...valid, page: "../etc" }).ok).toBe(false);
  });
});

describe("PromptsStore", () => {
  test("adds a queued record, persists it, and reloads it", async () => {
    const dir = await mkdtemp(join(tmpdir(), "prompts-"));
    const path = join(dir, "prompts.json");
    const store = new PromptsStore(path);
    const parsed = validateNewPrompt(valid);
    if (!parsed.ok) throw new Error("fixture invalid");
    const record = await store.add(parsed.value, new Date("2026-11-21T04:30:00Z"));
    expect(record.id).toMatch(/^prompt:/);
    expect(record.status).toBe("queued");
    expect(record.createdAt).toBe("2026-11-21T04:30:00.000Z");

    const onDisk = JSON.parse(await readFile(path, "utf8"));
    expect(onDisk.prompts[0].id).toBe(record.id);

    const reloaded = new PromptsStore(path);
    expect((await reloaded.get(record.id))?.prompt).toBe("reprioritize");
    expect((await reloaded.list({ page: "queue" })).length).toBe(1);
    expect((await reloaded.list({ page: "program" })).length).toBe(0);
    expect((await reloaded.list({ status: "answered" })).length).toBe(0);
  });

  test("a missing file is an empty store", async () => {
    const dir = await mkdtemp(join(tmpdir(), "prompts-"));
    expect(await new PromptsStore(join(dir, "none.json")).list()).toEqual([]);
  });
});

describe("renderPromptWidget", () => {
  test("posts to /api/prompt with scope, page and section carried", () => {
    const html = renderPromptWidget({ scope: "section", page: "queue", sectionRef: "item-7" });
    expect(html).toContain('action="/api/prompt"');
    expect(html).toContain('name="scope" value="section"');
    expect(html).toContain('name="sectionRef" value="item-7"');
    expect(html).toContain('<option value="pool:auto">');
  });

  test("escapes caller-supplied values", () => {
    const html = renderPromptWidget({ scope: "section", page: "queue", sectionRef: '"><script>' });
    expect(html).not.toContain("<script>");
    expect(html).toContain("&#34;&#62;");
  });
});
