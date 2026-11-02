import { Packr, Unpackr } from "msgpackr";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MAX_STEPS_MESSAGE_BYTES, MAX_STEPS_PER_MESSAGE, initCollab, stepBatch } from "./collab";

// Same MessagePack configuration as collab.ts
const packr = new Packr({ useRecords: false, structuredClone: true });
const unpackr = new Unpackr({ useRecords: false, structuredClone: true });

const MSG_AUTH = 8;
const MSG_AUTH_OK = 9;

/** Minimal WebSocket stand-in: records sent frames, lets the test drive
 * open/message/close by hand. No real network is involved. */
class FakeWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static instances: FakeWebSocket[] = [];

  readonly CONNECTING = 0;
  readonly OPEN = 1;
  readonly CLOSING = 2;
  readonly CLOSED = 3;

  readyState = FakeWebSocket.CONNECTING;
  binaryType = "";
  sent: unknown[] = [];
  onopen: (() => void) | null = null;
  onclose: ((event: { code: number; reason: string }) => void) | null = null;
  onerror: ((event: unknown) => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;

  constructor(readonly url: string) {
    FakeWebSocket.instances.push(this);
  }

  send(data: unknown): void {
    this.sent.push(data);
  }

  close(): void {
    this.readyState = FakeWebSocket.CLOSED;
  }

  // Test drivers -----------------------------------------------------------
  open(): void {
    this.readyState = FakeWebSocket.OPEN;
    this.onopen?.();
  }

  deliver(msg: unknown[]): void {
    const packed = packr.pack(msg);
    const buf = packed.buffer.slice(
      packed.byteOffset,
      packed.byteOffset + packed.byteLength,
    ) as ArrayBuffer;
    this.onmessage?.({ data: buf });
  }

  drop(): void {
    this.readyState = FakeWebSocket.CLOSED;
    // 1006 = abnormal closure (a dropped connection, not an intentional one)
    this.onclose?.({ code: 1006, reason: "dropped" });
  }

  firstFrame(): unknown {
    expect(this.sent.length).toBeGreaterThan(0);
    return unpackr.unpack(this.sent[0] as Uint8Array);
  }
}

function makeContainer(): HTMLElement {
  const el = document.createElement("div");
  document.body.appendChild(el);
  return el;
}

const liveConnections: { disconnect: () => void }[] = [];
function track<T extends { disconnect: () => void }>(conn: T): T {
  liveConnections.push(conn);
  return conn;
}

describe("initCollab token handling (#1463)", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    FakeWebSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeWebSocket);
  });

  afterEach(() => {
    while (liveConnections.length > 0) {
      liveConnections.pop()?.disconnect();
    }
    vi.unstubAllGlobals();
    vi.useRealTimers();
    document.body.innerHTML = "";
  });

  it("reconnect AUTH sends the AUTH_OK refreshed token, not the init token", () => {
    const onTokenRefresh = vi.fn();
    const conn = track(initCollab(
      "ws://example.test/collab",
      makeContainer(),
      "doc-1",
      "notes.md",
      "OLD_TOKEN_A",
      onTokenRefresh,
    ));

    const first = FakeWebSocket.instances[0];
    first.open();
    expect(first.firstFrame()).toEqual([MSG_AUTH, "OLD_TOKEN_A"]);

    // Server confirms identity and hands back a refreshed token
    first.deliver([MSG_AUTH_OK, "client-1", "alice", "NEW_TOKEN_B"]);
    expect(onTokenRefresh).toHaveBeenCalledWith("NEW_TOKEN_B");

    // Connection drops; the scheduled reconnect creates a second socket
    first.drop();
    vi.advanceTimersByTime(1_000);
    expect(FakeWebSocket.instances.length).toBe(2);

    const second = FakeWebSocket.instances[1];
    second.open();
    expect(second.firstFrame()).toEqual([MSG_AUTH, "NEW_TOKEN_B"]);

    conn.disconnect();
  });

  it("reconnect without any refresh re-sends the init token", () => {
    const conn = track(initCollab(
      "ws://example.test/collab",
      makeContainer(),
      "doc-1",
      "notes.md",
      "ONLY_TOKEN",
      undefined,
    ));

    const first = FakeWebSocket.instances[0];
    first.open();
    expect(first.firstFrame()).toEqual([MSG_AUTH, "ONLY_TOKEN"]);

    first.drop();
    vi.advanceTimersByTime(1_000);
    const second = FakeWebSocket.instances[1];
    second.open();
    expect(second.firstFrame()).toEqual([MSG_AUTH, "ONLY_TOKEN"]);

    conn.disconnect();
  });

  it("anonymous connect sends no AUTH frame, before or after reconnect", () => {
    const conn = track(initCollab(
      "ws://example.test/collab",
      makeContainer(),
      "doc-1",
      "notes.md",
      null,
      undefined,
    ));

    const first = FakeWebSocket.instances[0];
    first.open();
    expect(first.sent.length).toBe(0);

    first.drop();
    vi.advanceTimersByTime(1_000);
    const second = FakeWebSocket.instances[1];
    second.open();
    expect(second.sent.length).toBe(0);

    conn.disconnect();
  });
});

describe("stepBatch (client side of the server step cap, #1965)", () => {
  it("sends every step when the batch is at or under the cap", () => {
    const steps = [1, 2, 3];
    expect(stepBatch(steps)).toEqual([1, 2, 3]);
    const atCap = Array.from({ length: MAX_STEPS_PER_MESSAGE }, (_, i) => i);
    expect(stepBatch(atCap)).toHaveLength(MAX_STEPS_PER_MESSAGE);
  });

  it("chunks an oversized transaction to the cap, preserving order", () => {
    // Indenting >1000 lines legitimately produces >1000 steps; the first
    // message must carry only the cap and leave the rest pending, or the
    // server rejection wedges the session (Codex P2 on PR #4051).
    const steps = Array.from({ length: MAX_STEPS_PER_MESSAGE + 250 }, (_, i) => i);
    const batch = stepBatch(steps);
    expect(batch).toHaveLength(MAX_STEPS_PER_MESSAGE);
    expect(batch[0]).toBe(0);
    expect(batch[MAX_STEPS_PER_MESSAGE - 1]).toBe(MAX_STEPS_PER_MESSAGE - 1);
    expect(steps).toHaveLength(MAX_STEPS_PER_MESSAGE + 250);
  });

  it("returns an empty batch for no pending steps", () => {
    expect(stepBatch([])).toEqual([]);
  });

  it("chunks by encoded size as well as count (Codex P2 on PR #4051)", () => {
    // 1,000 small steps fit the count cap, but medium steps can encode to
    // far more than the 1 MiB server frame cap; the batch must stop at
    // the byte budget and leave the remainder pending, in order.
    const stepBytes = 4096;
    const steps = Array.from({ length: 1000 }, (_, i) => i);
    const batch = stepBatch(steps, () => stepBytes);
    const expected = Math.floor(MAX_STEPS_MESSAGE_BYTES / stepBytes);
    expect(batch).toHaveLength(expected);
    expect(batch[0]).toBe(0);
    expect(batch[expected - 1]).toBe(expected - 1);
    expect(steps).toHaveLength(1000);
  });

  it("withholds a single step that alone exceeds the byte budget", () => {
    // A >1 MiB paste can be one replace step; it can never be sent within
    // the server cap, so the batch is empty and the caller keeps the edit
    // local instead of triggering a server refusal + reconnect discard.
    const steps = ["huge", "later"];
    const batch = stepBatch(steps, (s) => (s === "huge" ? MAX_STEPS_MESSAGE_BYTES + 1 : 1));
    expect(batch).toEqual([]);
  });

  it("includes a first step that exactly fits the byte budget", () => {
    const steps = ["exact", "later"];
    const batch = stepBatch(steps, (s) => (s === "exact" ? MAX_STEPS_MESSAGE_BYTES : 1));
    expect(batch).toEqual(["exact"]);
  });
});
