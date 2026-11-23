import { describe, expect, test } from "bun:test";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PromptsStore, validateNewPrompt } from "./prompts.ts";
import { createPromptWsSession, type PromptWsSocket } from "./prompt-ws.ts";

class FakeSocket implements PromptWsSocket {
  sent: string[] = [];
  closedWith: { code?: number; reason?: string } | null = null;
  send(data: string): void {
    this.sent.push(data);
  }
  close(code?: number, reason?: string): void {
    this.closedWith = { code, reason };
  }
  messages(): Record<string, unknown>[] {
    return this.sent.map((s) => JSON.parse(s) as Record<string, unknown>);
  }
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function newStoreWithPrompt() {
  const dir = await mkdtemp(join(tmpdir(), "prompt-ws-"));
  const store = new PromptsStore(join(dir, "prompts.json"));
  const parsed = validateNewPrompt({
    scope: "page",
    target: "claude",
    mode: "sync-ws",
    page: "queue",
    sectionRef: null,
    prompt: "status?",
  });
  if (!parsed.ok) throw new Error("fixture invalid");
  const record = await store.add(parsed.value);
  return { store, record };
}

describe("createPromptWsSession (S3 sync-ws)", () => {
  test("streams state changes and closes on completion", async () => {
    const { store, record } = await newStoreWithPrompt();
    const socket = new FakeSocket();
    const session = createPromptWsSession({ store, promptId: record.id, socket, timeoutMs: 2000, pollMs: 5 });
    await session.start();
    expect(socket.messages()).toEqual([{ id: record.id, status: "queued" }]);
    await store.update(record.id, { status: "running" });
    await sleep(40);
    expect(socket.messages().at(-1)).toEqual({ id: record.id, status: "running" });
    await store.update(record.id, { status: "answered", answer: "done" });
    await sleep(40);
    expect(socket.messages().at(-1)).toEqual({ id: record.id, status: "answered", answer: "done" });
    expect(socket.closedWith?.code).toBe(1000);
    session.stop();
  });

  test("an already-answered prompt sends its final state and closes at once", async () => {
    const { store, record } = await newStoreWithPrompt();
    await store.update(record.id, { status: "answered", answer: "ready" });
    const socket = new FakeSocket();
    const session = createPromptWsSession({ store, promptId: record.id, socket, timeoutMs: 2000, pollMs: 5 });
    await session.start();
    expect(socket.messages()).toEqual([{ id: record.id, status: "answered", answer: "ready" }]);
    expect(socket.closedWith?.code).toBe(1000);
    session.stop();
  });

  test("a failed prompt closes with the failure state", async () => {
    const { store, record } = await newStoreWithPrompt();
    const socket = new FakeSocket();
    const session = createPromptWsSession({ store, promptId: record.id, socket, timeoutMs: 2000, pollMs: 5 });
    await session.start();
    await store.update(record.id, { status: "failed", error: "boom" });
    await sleep(40);
    expect(socket.messages().at(-1)).toEqual({ id: record.id, status: "failed", error: "boom" });
    expect(socket.closedWith?.code).toBe(1000);
    session.stop();
  });

  test("an unanswered prompt times out with a typed message and closes", async () => {
    const { store, record } = await newStoreWithPrompt();
    const socket = new FakeSocket();
    const session = createPromptWsSession({ store, promptId: record.id, socket, timeoutMs: 60, pollMs: 5 });
    await session.start();
    await sleep(200);
    const last = socket.messages().at(-1);
    expect(last?.status).toBe("timeout");
    expect(last?.timeoutMs).toBe(60);
    expect(socket.closedWith?.code).toBe(1000);
    session.stop();
  });

  test("an unknown prompt id reports not-found and closes", async () => {
    const { store } = await newStoreWithPrompt();
    const socket = new FakeSocket();
    const session = createPromptWsSession({ store, promptId: "prompt:missing", socket, timeoutMs: 2000, pollMs: 5 });
    await session.start();
    expect(socket.messages()).toEqual([{ id: "prompt:missing", status: "not-found", error: "prompt not found" }]);
    expect(socket.closedWith?.code).toBe(1008);
    session.stop();
  });
});
