import { Packr, Unpackr } from "msgpackr";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  MAX_STEPS_MESSAGE_BYTES,
  MAX_STEPS_PER_MESSAGE,
  buildCollabWsUrl,
  initCollab,
  stepBatch,
  stepWireSize,
} from "./collab";
import { getSendableSteps, getVersion } from "./editor";

// Same MessagePack configuration as collab.ts
const packr = new Packr({ useRecords: false, structuredClone: true });
const unpackr = new Unpackr({ useRecords: false, structuredClone: true });

const MSG_AUTH = 8;
const MSG_AUTH_OK = 9;
const MSG_INIT = 0;

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

  deliver(msg: unknown): void {
    const packed = packr.pack(msg);
    const buf = packed.buffer.slice(packed.byteOffset, packed.byteOffset + packed.byteLength) as ArrayBuffer;
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

function openCollab(onEditorReady = vi.fn()) {
  const conn = track(
    initCollab(
      "ws://example.test/collab",
      makeContainer(),
      "doc-1",
      "notes.md",
      null,
      undefined,
      undefined,
      onEditorReady,
    ),
  );
  const socket = FakeWebSocket.instances.at(-1)!;
  socket.open();
  return { conn, socket, onEditorReady };
}

const validInit = () => [MSG_INIT, 0, { type: "doc", content: [{ type: "paragraph" }] }, "raw"];

describe("initCollab protocol handling (#1463, #2824)", () => {
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
    const conn = track(
      initCollab("ws://example.test/collab", makeContainer(), "doc-1", "notes.md", "OLD_TOKEN_A", onTokenRefresh),
    );

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
    const conn = track(
      initCollab("ws://example.test/collab", makeContainer(), "doc-1", "notes.md", "ONLY_TOKEN", undefined),
    );

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
    const conn = track(initCollab("ws://example.test/collab", makeContainer(), "doc-1", "notes.md", null, undefined));

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
    const conn = track(
      initCollab("ws://example.test/collab", makeContainer(), "doc-1", "notes.md", null, undefined, (status) =>
        statuses.push(status),
      ),
    );

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
    const conn = track(
      initCollab("ws://example.test/collab", makeContainer(), "doc-1", "notes.md", null, undefined, (status) =>
        statuses.push(status),
      ),
    );

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
    const conn = track(initCollab("ws://example.test/collab", makeContainer(), "doc-1"));
    const socket = FakeWebSocket.instances[0];

    socket.open();
    conn.disconnect();
    // A failed close handshake can surface as an abnormal close after the
    // client has already requested a normal, intentional close.
    socket.drop();
    vi.advanceTimersByTime(10_000);

    expect(FakeWebSocket.instances).toHaveLength(1);
  });

  it("drops non-array frames and still initializes from a later valid INIT (#2824)", () => {
    const onEditorReady = vi.fn();
    const { conn, socket } = openCollab(onEditorReady);
    vi.spyOn(console, "error").mockImplementation(() => {});

    for (const frame of [null, { type: MSG_INIT }, "not an array"]) {
      socket.deliver(frame);
      expect(conn.editor).toBeNull();
    }
    expect(onEditorReady).not.toHaveBeenCalled();

    socket.deliver(validInit());
    expect(conn.editor).not.toBeNull();
    expect(onEditorReady).toHaveBeenCalledOnce();
  });

  it("rejects malformed INIT version/document fields before editor initialization (#2824)", () => {
    const onEditorReady = vi.fn();
    const { conn, socket } = openCollab(onEditorReady);
    vi.spyOn(console, "error").mockImplementation(() => {});

    for (const frame of [
      [MSG_INIT],
      [MSG_INIT, "0", { type: "doc", content: [] }, "raw"],
      [MSG_INIT, 0, null, "raw"],
      [MSG_INIT, 0, [], "raw"],
      [MSG_INIT, 0, {}, "raw"],
    ]) {
      socket.deliver(frame);
      expect(conn.editor).toBeNull();
      expect(onEditorReady).not.toHaveBeenCalled();
    }

    socket.deliver(validInit());
    expect(conn.editor).not.toBeNull();
    expect(onEditorReady).toHaveBeenCalledOnce();
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

describe("large text insertion collaboration (#4119)", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    FakeWebSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeWebSocket);
  });

  afterEach(() => {
    while (liveConnections.length > 0) liveConnections.pop()?.disconnect();
    vi.unstubAllGlobals();
    vi.useRealTimers();
    document.body.innerHTML = "";
  });

  it("sends and confirms all bounded steps from a single oversized paste", () => {
    const { conn, socket } = openCollab();
    socket.deliver([MSG_INIT, 0, { type: "doc", content: [{ type: "code_block" }] }, "raw"]);
    expect(conn.editor).not.toBeNull();

    const view = conn.editor!.view;
    const text = "x".repeat(1_100_000);
    view.dispatch(view.state.tr.insertText(text, 1));

    let confirmed = 0;
    for (let frameIndex = 0; frameIndex < 100; frameIndex++) {
      const frames = socket.sent.map((data) => unpackr.unpack(data as Uint8Array) as unknown[]);
      const stepFrames = frames.filter((frame) => frame[0] === 1);
      const frame = stepFrames[frameIndex];
      if (!frame) break;

      const steps = frame[2] as unknown[];
      const clientID = frame[3] as number;
      expect(steps.length).toBeGreaterThan(0);
      expect(steps.length).toBeLessThanOrEqual(MAX_STEPS_PER_MESSAGE);
      expect(steps.every((step) => stepWireSize(step) <= MAX_STEPS_MESSAGE_BYTES)).toBe(true);

      confirmed += steps.length;
      socket.deliver([2, steps, steps.map(() => clientID)]);
    }

    const allFrames = socket.sent.map((data) => unpackr.unpack(data as Uint8Array) as unknown[]);
    const stepFrameData = socket.sent.filter((data) => (unpackr.unpack(data as Uint8Array) as unknown[])[0] === 1);
    const sentSteps = allFrames.filter((frame) => frame[0] === 1).flatMap((frame) => frame[2] as unknown[]);
    expect(stepFrameData.every((data) => (data as Uint8Array).byteLength <= 1024 * 1024)).toBe(true);
    expect(sentSteps.length).toBeGreaterThan(1);
    expect(sentSteps.map((step) => (step as { slice: { content: Array<{ text?: string }> } }).slice.content
      .map((node) => node.text ?? "").join("")).join("")).toBe(text);
    expect(confirmed).toBe(sentSteps.length);
    expect(getSendableSteps(view)).toBeNull();
    expect(getVersion(view.state)).toBe(confirmed);
    expect(view.state.doc.textContent).toBe(text);
  });
});

describe("buildCollabWsUrl (#2830)", () => {
  // [docId, expected single path segment] — the segment must decode
  // back to the exact docId, and plain names must be untouched.
  const cases: Array<[string, string]> = [
    ["notes.md", "notes.md"],
    ["a/b.md", "a%2Fb.md"],
    ["a#b.md", "a%23b.md"],
    ["a?b.md", "a%3Fb.md"],
    ["a%20b.md", "a%2520b.md"],
    ["100%.md", "100%25.md"],
    ["my notes.md", "my%20notes.md"],
    ["caf\u00e9.md", "caf%C3%A9.md"],
  ];
  it.each(cases)("docId %s -> one segment %s that decodes back exactly", (docId, segment) => {
    const url = new URL(buildCollabWsUrl("ws:", "example.test", docId));
    expect(url.pathname).toBe(`/ws/collab/${segment}`);
    const parts = url.pathname.split("/");
    expect(parts).toHaveLength(4); // "", "ws", "collab", segment
    expect(decodeURIComponent(parts[3])).toBe(docId);
  });

  it("keeps the filename query parameter intact for a # name", () => {
    vi.useFakeTimers();
    FakeWebSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const conn = track(
      initCollab(buildCollabWsUrl("ws:", "example.test", "a#b.md"), makeContainer(), "a#b.md", "a#b.md", null),
    );
    const socket = FakeWebSocket.instances.at(-1)!;
    const url = new URL(socket.url);
    expect(decodeURIComponent(url.pathname.split("/")[3])).toBe("a#b.md");
    expect(url.searchParams.get("filename")).toBe("a#b.md");
    conn.disconnect();
    vi.useRealTimers();
  });
});
