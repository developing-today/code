import { Packr, Unpackr } from "msgpackr";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { initCollab } from "./collab";

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

  it("caps repeated open-then-drop failures until Init succeeds", () => {
    const statuses: string[] = [];
    const conn = track(initCollab(
      "ws://example.test/collab",
      makeContainer(),
      "doc-1",
      "notes.md",
      null,
      undefined,
      (status) => statuses.push(status),
    ));

    let socket = FakeWebSocket.instances[0];
    // The initial connection plus ten reconnects all reach OPEN, but the
    // server drops each one before sending Init.
    for (let i = 0; i < 11; i++) {
      socket.open();
      socket.drop();
      if (i < 10) {
        vi.advanceTimersToNextTimer();
        socket = FakeWebSocket.instances.at(-1)!;
      }
    }

    expect(FakeWebSocket.instances).toHaveLength(11);
    expect(statuses.at(-1)).toBe("error");
    conn.disconnect();
  });

  it("starts a fresh retry budget after receiving Init", () => {
    const statuses: string[] = [];
    const conn = track(initCollab(
      "ws://example.test/collab",
      makeContainer(),
      "doc-1",
      "notes.md",
      null,
      undefined,
      (status) => statuses.push(status),
    ));

    let socket = FakeWebSocket.instances[0];
    for (let i = 0; i < 10; i++) {
      socket.open();
      socket.drop();
      vi.advanceTimersToNextTimer();
      socket = FakeWebSocket.instances.at(-1)!;
    }

    socket.open();
    socket.deliver([0, 0, { type: "doc", content: [{ type: "paragraph" }] }, "raw"]);
    socket.drop();
    vi.advanceTimersToNextTimer();
    socket = FakeWebSocket.instances.at(-1)!;

    // Ten failures after successful initialization may still schedule the
    // tenth retry; the following failed connection reaches the cap.
    for (let i = 0; i < 9; i++) {
      socket.open();
      socket.drop();
      if (i < 8) {
        vi.advanceTimersToNextTimer();
        socket = FakeWebSocket.instances.at(-1)!;
      }
    }
    expect(statuses.at(-1)).toBe("connecting");

    vi.advanceTimersToNextTimer();
    socket = FakeWebSocket.instances.at(-1)!;
    socket.open();
    socket.drop();
    expect(statuses.at(-1)).toBe("error");
    conn.disconnect();
  });

  it("does not reconnect after an intentional disconnect", () => {
    const conn = track(initCollab(
      "ws://example.test/collab",
      makeContainer(),
      "doc-1",
    ));
    const socket = FakeWebSocket.instances[0];

    socket.open();
    conn.disconnect();
    // A failed close handshake can surface as an abnormal close after the
    // client has already requested a normal, intentional close.
    socket.drop();
    vi.advanceTimersByTime(10_000);

    expect(FakeWebSocket.instances).toHaveLength(1);
  });
});
