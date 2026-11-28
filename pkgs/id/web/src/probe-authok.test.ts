import { Packr, Unpackr } from "msgpackr";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { initCollab } from "./collab";

const packr = new Packr({ useRecords: false, structuredClone: true });
const unpackr = new Unpackr({ useRecords: false, structuredClone: true });

const MSG_AUTH = 8;
const MSG_AUTH_OK = 9;
const MSG_INIT = 0;
const MSG_NEW_VERSION = 7;

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
  open(): void {
    this.readyState = FakeWebSocket.OPEN;
    this.onopen?.();
  }
  deliver(msg: unknown): void {
    const packed = packr.pack(msg);
    const buf = packed.buffer.slice(packed.byteOffset, packed.byteOffset + packed.byteLength) as ArrayBuffer;
    this.onmessage?.({ data: buf });
  }
  drop(): void {
    this.readyState = FakeWebSocket.CLOSED;
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

const validInit = () => [MSG_INIT, 0, { type: "doc", content: [{ type: "paragraph" }] }, "raw"];

describe("probe: AUTH_OK / NEW_VERSION field shape", () => {
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

  it("control: well-formed AUTH_OK string token refreshes and is re-sent on reconnect", () => {
    const onTokenRefresh = vi.fn();
    const conn = track(
      initCollab("ws://example.test/collab", makeContainer(), "doc-1", "notes.md", "OLD_TOKEN_A", onTokenRefresh),
    );
    const first = FakeWebSocket.instances[0];
    first.open();
    expect(first.firstFrame()).toEqual([MSG_AUTH, "OLD_TOKEN_A"]);
    first.deliver([MSG_AUTH_OK, "client-1", "alice", "NEW_TOKEN_B"]);
    expect(onTokenRefresh).toHaveBeenCalledWith("NEW_TOKEN_B");
    first.drop();
    vi.advanceTimersByTime(1_000);
    const second = FakeWebSocket.instances[1];
    second.open();
    expect(second.firstFrame()).toEqual([MSG_AUTH, "NEW_TOKEN_B"]);
    conn.disconnect();
  });

  it("BUG: AUTH_OK with a numeric refreshed token is adopted verbatim", () => {
    const onTokenRefresh = vi.fn();
    const conn = track(
      initCollab("ws://example.test/collab", makeContainer(), "doc-1", "notes.md", "OLD_TOKEN_A", onTokenRefresh),
    );
    const first = FakeWebSocket.instances[0];
    first.open();
    first.deliver(validInit());
    first.deliver([MSG_AUTH_OK, "client-1", "alice", 12345]);
    // What did the refresh callback receive, and of what type?
    const received = onTokenRefresh.mock.calls.at(-1)?.[0];
    console.log("PROBE numeric token -> onTokenRefresh received:", JSON.stringify(received), typeof received);
    first.drop();
    vi.advanceTimersByTime(1_000);
    const second = FakeWebSocket.instances[1];
    second.open();
    console.log("PROBE numeric token -> reconnect AUTH frame:", JSON.stringify(second.firstFrame()));
    conn.disconnect();
  });

  it("BUG: AUTH_OK with an object refreshed token is adopted verbatim", () => {
    const onTokenRefresh = vi.fn();
    const conn = track(
      initCollab("ws://example.test/collab", makeContainer(), "doc-1", "notes.md", "OLD_TOKEN_A", onTokenRefresh),
    );
    const first = FakeWebSocket.instances[0];
    first.open();
    first.deliver(validInit());
    first.deliver([MSG_AUTH_OK, "client-1", "alice", { token: "nested" }]);
    const received = onTokenRefresh.mock.calls.at(-1)?.[0];
    console.log(
      "PROBE object token -> onTokenRefresh received:",
      JSON.stringify(received),
      typeof received,
      "String():",
      String(received),
    );
    conn.disconnect();
  });

  it("BUG: NEW_VERSION with a non-string hash reaches onNewVersion verbatim", () => {
    const onNewVersion = vi.fn();
    const conn = track(
      initCollab(
        "ws://example.test/collab",
        makeContainer(),
        "doc-1",
        "notes.md",
        null,
        undefined,
        undefined,
        undefined,
        onNewVersion,
      ),
    );
    const first = FakeWebSocket.instances[0];
    first.open();
    first.deliver(validInit());
    first.deliver([MSG_NEW_VERSION, { x: 1 }, "notes.md"]);
    const received = onNewVersion.mock.calls.at(-1);
    console.log(
      "PROBE object hash -> onNewVersion received:",
      JSON.stringify(received),
      "String(hash):",
      String(received?.[0]),
    );
    conn.disconnect();
  });

  it("control: NEW_VERSION with a string hash reaches onNewVersion verbatim", () => {
    const onNewVersion = vi.fn();
    const conn = track(
      initCollab(
        "ws://example.test/collab",
        makeContainer(),
        "doc-1",
        "notes.md",
        null,
        undefined,
        undefined,
        undefined,
        onNewVersion,
      ),
    );
    const first = FakeWebSocket.instances[0];
    first.open();
    first.deliver(validInit());
    first.deliver([MSG_NEW_VERSION, "abc123hash", "notes.md"]);
    expect(onNewVersion).toHaveBeenCalledWith("abc123hash", "notes.md");
    conn.disconnect();
  });
});
