import { describe, expect, test } from "bun:test";
import { mkdtemp, mkdir, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  PromptsStore,
  syncBlockHttpResult,
  validateNewPrompt,
  type NewPrompt,
  type PromptRecord,
  type PromptWaitResult,
} from "./prompts.ts";
import { renderPromptWidget } from "./prompt-widget.ts";
import { loadConfig } from "./config.ts";

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
    expect(validateNewPrompt({ ...valid, mode: "sync" }).ok).toBe(false);
  });

  test("accepts the S3 sync modes", () => {
    expect(validateNewPrompt({ ...valid, mode: "sync-block" }).ok).toBe(true);
    expect(validateNewPrompt({ ...valid, mode: "sync-ws" }).ok).toBe(true);
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

  test("accepts the decision target with its design 3.4 fields", () => {
    const decision = {
      contextLevel: "medium" as const,
      advancedConstraints: [{ when: { field: "a", eq: 1 }, then: { field: "b", required: true } }],
      contextGenerator: "model:claude-haiku-5-5",
    };
    const r = validateNewPrompt({ ...valid, target: "decision", decision });
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.value.decision).toEqual(decision);
  });

  test("accepts a null contextGenerator and an empty constraint list", () => {
    const r = validateNewPrompt({
      ...valid,
      target: "decision",
      decision: { contextLevel: "high", advancedConstraints: [], contextGenerator: null },
    });
    expect(r.ok).toBe(true);
  });

  test("rejects a decision target with a missing or malformed decision object", () => {
    expect(validateNewPrompt({ ...valid, target: "decision" }).ok).toBe(false);
    expect(
      validateNewPrompt({
        ...valid,
        target: "decision",
        decision: { contextLevel: "extreme", advancedConstraints: [], contextGenerator: null },
      }).ok,
    ).toBe(false);
    expect(
      validateNewPrompt({
        ...valid,
        target: "decision",
        decision: { contextLevel: "low", advancedConstraints: ["x"], contextGenerator: null },
      }).ok,
    ).toBe(false);
    expect(
      validateNewPrompt({
        ...valid,
        target: "decision",
        decision: { contextLevel: "low", advancedConstraints: [], contextGenerator: "gpt" },
      }).ok,
    ).toBe(false);
  });

  test("ignores a decision object on non-decision targets", () => {
    const r = validateNewPrompt({
      ...valid,
      decision: { contextLevel: "low", advancedConstraints: [], contextGenerator: null },
    });
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.value.decision).toBeUndefined();
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

  test("concurrent adds on a missing file all settle and persist exactly", async () => {
    const dir = await mkdtemp(join(tmpdir(), "prompts-"));
    const path = join(dir, "prompts.json");
    const store = new PromptsStore(path);
    const parsed = validateNewPrompt(valid);
    if (!parsed.ok) throw new Error("fixture invalid");
    const results = await Promise.allSettled(
      Array.from({ length: 20 }, (_, i) => store.add({ ...parsed.value, prompt: `p${i}` })),
    );
    const fulfilled = results.flatMap((r) => (r.status === "fulfilled" ? [r.value] : []));
    expect(fulfilled.length).toBe(20);
    const onDisk = JSON.parse(await readFile(path, "utf8"));
    expect(onDisk.prompts.map((p: { id: string }) => p.id).sort()).toEqual(fulfilled.map((p) => p.id).sort());
  });

  test("a failed write rejects only its own caller and does not poison later writes", async () => {
    const dir = await mkdtemp(join(tmpdir(), "prompts-"));
    const path = join(dir, "prompts.json");
    const store = new PromptsStore(path);
    const parsed = validateNewPrompt(valid);
    if (!parsed.ok) throw new Error("fixture invalid");
    // Load first (missing file is an empty store), then put a directory
    // at the target path so the queued write fails at rename.
    await store.list();
    await mkdir(path);
    await expect(store.add(parsed.value)).rejects.toThrow();
    await rm(path, { recursive: true });
    const record = await store.add(parsed.value);
    const onDisk = JSON.parse(await readFile(path, "utf8"));
    expect(onDisk.prompts.map((p: { id: string }) => p.id)).toEqual([record.id]);
  });
});

describe("PromptsStore.waitForAnswer (S3 sync-block)", () => {
  async function newStore() {
    const dir = await mkdtemp(join(tmpdir(), "prompts-wait-"));
    const store = new PromptsStore(join(dir, "prompts.json"));
    const parsed = validateNewPrompt({ ...valid, mode: "sync-block" });
    if (!parsed.ok) throw new Error("fixture invalid");
    const record = await store.add(parsed.value);
    return { store, record };
  }

  test("resolves at once for an already-answered prompt", async () => {
    const { store, record } = await newStore();
    await store.update(record.id, { status: "answered", answer: "ready" });
    const wait = await store.waitForAnswer(record.id, { timeoutMs: 1000, pollMs: 5 });
    expect(wait.outcome).toBe("answered");
    if (wait.outcome === "answered") expect(wait.record.answer).toBe("ready");
  });

  test("resolves when the answer lands mid-wait", async () => {
    const { store, record } = await newStore();
    const waiting = store.waitForAnswer(record.id, { timeoutMs: 2000, pollMs: 5 });
    await new Promise((r) => setTimeout(r, 20));
    await store.update(record.id, { status: "answered", answer: "late answer" });
    const wait = await waiting;
    expect(wait.outcome).toBe("answered");
    if (wait.outcome === "answered") expect(wait.record.answer).toBe("late answer");
  });

  test("resolves as failed when the prompt fails", async () => {
    const { store, record } = await newStore();
    const waiting = store.waitForAnswer(record.id, { timeoutMs: 2000, pollMs: 5 });
    await store.update(record.id, { status: "failed", error: "boom" });
    const wait = await waiting;
    expect(wait.outcome).toBe("failed");
    if (wait.outcome === "failed") expect(wait.record.error).toBe("boom");
  });

  test("times out with the still-queued record when no answer lands", async () => {
    const { store, record } = await newStore();
    const wait = await store.waitForAnswer(record.id, { timeoutMs: 60, pollMs: 5 });
    expect(wait.outcome).toBe("timeout");
    if (wait.outcome === "timeout") expect(wait.record.status).toBe("queued");
  });

  test("reports not-found for an unknown id", async () => {
    const { store } = await newStore();
    const wait = await store.waitForAnswer("prompt:missing", { timeoutMs: 60, pollMs: 5 });
    expect(wait).toEqual({ outcome: "not-found" });
  });
});

describe("syncBlockHttpResult (S3 response shapes)", () => {
  const base: PromptRecord = {
    scope: "page",
    target: "claude",
    mode: "sync-block",
    page: "queue",
    sectionRef: null,
    prompt: "reprioritize",
    id: "prompt:1",
    threadId: "prompt:1",
    status: "queued",
    createdAt: "2026-11-23T00:00:00.000Z",
    archived: false,
    read: false,
    replyTo: null,
    refs: [],
    notify: false,
    attachments: [],
  };

  test("answered maps to the design §3.7 body at 200", () => {
    const wait: PromptWaitResult = {
      outcome: "answered",
      record: { ...base, status: "answered", answer: "the answer" },
    };
    const result = syncBlockHttpResult(wait, 120000);
    expect(result.status).toBe(200);
    expect(result.body).toEqual({ id: "prompt:1", status: "answered", answer: "the answer", attachments: [] });
  });

  test("failed maps to a typed 200 result carrying the error", () => {
    const wait: PromptWaitResult = { outcome: "failed", record: { ...base, status: "failed", error: "boom" } };
    const result = syncBlockHttpResult(wait, 120000);
    expect(result.status).toBe(200);
    expect(result.body).toEqual({ id: "prompt:1", status: "failed", error: "boom" });
  });

  test("timeout maps to a typed 504 result naming the configured wait", () => {
    const wait: PromptWaitResult = { outcome: "timeout", record: { ...base } };
    const result = syncBlockHttpResult(wait, 120000);
    expect(result.status).toBe(504);
    expect(result.body).toEqual({
      id: "prompt:1",
      status: "timeout",
      error: "prompt not answered within 120000 ms",
      timeoutMs: 120000,
    });
  });

  test("not-found maps to 404", () => {
    const result = syncBlockHttpResult({ outcome: "not-found" }, 120000);
    expect(result.status).toBe(404);
  });
});

describe("loadConfig prompt sync timeout (S3)", () => {
  test("defaults to the design's 120s and honors PM_AGG_PROMPT_SYNC_TIMEOUT_MS", () => {
    expect(loadConfig({ PM_AGG_DEV: "1" }).promptSyncTimeoutMs).toBe(120000);
    expect(loadConfig({ PM_AGG_DEV: "1", PM_AGG_PROMPT_SYNC_TIMEOUT_MS: "45000" }).promptSyncTimeoutMs).toBe(45000);
    expect(() => loadConfig({ PM_AGG_DEV: "1", PM_AGG_PROMPT_SYNC_TIMEOUT_MS: "soon" })).toThrow();
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
