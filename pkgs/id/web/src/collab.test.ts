import { Packr } from "msgpackr";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { initCollab } from "./collab";

/**
 * Regression tests for the reconnect attempt cap (#1368).
 *
 * The cap in scheduleReconnect() counts attempts, but the counter used to be
 * reset in socket.onopen. A server that accepts the WebSocket handshake and
 * then drops the connection (abnormal close, no Init ever sent) therefore
 * restarted every cycle at "attempt 1/10" and the client reconnected forever
 * without ever surfacing the "error" status. The counter now resets only
 * when an Init message establishes a session.
 */

class FakeWebSocket {
  static OPEN = 1;
  static CONNECTING = 0;
  static CLOSED = 3;
  static instances: FakeWebSocket[] = [];
  readyState = 0;
  binaryType = "";
  onopen: (() => void) | null = null;
  onclose: ((e: { code: number; reason: string }) => void) | null = null;
  onerror: ((e: unknown) => void) | null = null;
  onmessage: ((e: { data: unknown }) => void) | null = null;
  sent: unknown[] = [];
  constructor(public url: string) {
    FakeWebSocket.instances.push(this);
  }
  send(d: unknown) {
    this.sent.push(d);
  }
  close(code?: number, reason?: string) {
    this.readyState = 3;
    this.onclose?.({ code: code ?? 1000, reason: reason ?? "" });
  }
  open() {
    this.readyState = 1;
    this.onopen?.();
  }
  drop() {
    this.readyState = 3;
    this.onclose?.({ code: 1006, reason: "abnormal" });
  }
  receiveInit() {
    const packr = new Packr({ useRecords: false, structuredClone: true });
    const doc = { type: "doc", content: [{ type: "code_block", content: [{ type: "text", text: "hi" }] }] };
    const data = packr.pack([0, 0, doc, "raw"]);
    const buf = data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength) as ArrayBuffer;
    this.onmessage?.({ data: buf });
  }
}

function latest(): FakeWebSocket {
  return FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
}

/** Advance fake time in small steps until the scheduled reconnect creates a new socket (or time runs out). Small steps matter: a freshly created socket has a 2s connect timeout that must not fire before the test opens it. */
function waitForNextSocket() {
  const before = FakeWebSocket.instances.length;
  for (let t = 0; t < 6000 && FakeWebSocket.instances.length === before; t += 100) {
    vi.advanceTimersByTime(100);
  }
}

/** One accept-then-drop cycle, then wait for the reconnect socket to appear. */
function cycle() {
  const ws = latest();
  ws.open();
  ws.drop();
  waitForNextSocket();
}

describe("initCollab reconnect cap", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    FakeWebSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeWebSocket);
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("gives up with status error after 10 attempts against an accept-then-drop server", () => {
    const statuses: string[] = [];
    const container = document.createElement("div");
    const conn = initCollab("ws://localhost:1/ws", container, "doc1", "a.txt", null, undefined, (s) => statuses.push(s));

    // Initial socket + 10 reconnect sockets; the 11th scheduleReconnect call
    // must hit the cap instead of creating a 12th socket.
    for (let i = 0; i < 15; i++) cycle();

    expect(FakeWebSocket.instances.length).toBe(11);
    expect(statuses[statuses.length - 1]).toBe("error");
    conn.disconnect();
  });

  it("resets the attempt counter once an Init establishes a session", () => {
    const statuses: string[] = [];
    const container = document.createElement("div");
    const conn = initCollab("ws://localhost:1/ws", container, "doc1", "a.txt", null, undefined, (s) => statuses.push(s));

    // Burn 5 attempts against the dropping server.
    for (let i = 0; i < 5; i++) cycle();
    expect(FakeWebSocket.instances.length).toBe(6);

    // Next connection actually establishes a session (Init received), then drops.
    const ws = latest();
    ws.open();
    ws.receiveInit();
    ws.drop();
    waitForNextSocket();

    // Because Init reset the counter, that drop scheduled a fresh attempt 1
    // instead of continuing toward the cap: a new socket exists and the
    // client has not given up.
    expect(FakeWebSocket.instances.length).toBe(7);
    expect(statuses).not.toContain("error");
    conn.disconnect();
  });
});
