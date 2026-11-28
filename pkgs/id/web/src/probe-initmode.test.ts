import { Packr } from "msgpackr";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { initCollab } from "./collab";

const packr = new Packr({ useRecords: false, structuredClone: true });
const MSG_INIT = 0;

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
}

function makeContainer(): HTMLElement {
  const el = document.createElement("div");
  document.body.appendChild(el);
  return el;
}

const live: { disconnect: () => void }[] = [];

function openCollab(onEditorReady: (inst: unknown) => void) {
  const conn = initCollab("ws://example.test/collab", makeContainer(), "doc-1", "notes.md", null, undefined, undefined, onEditorReady as never);
  live.push(conn);
  const socket = FakeWebSocket.instances.at(-1)!;
  socket.open();
  return { conn, socket };
}

const rawDoc = { type: "doc", content: [{ type: "code_block", content: [{ type: "text", text: "hello raw" }] }] };
const richDoc = { type: "doc", content: [{ type: "paragraph", content: [{ type: "text", text: "hello rich" }] }] };

describe("probe: INIT mode shape", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    FakeWebSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeWebSocket);
  });
  afterEach(() => {
    while (live.length > 0) live.pop()?.disconnect();
    vi.unstubAllGlobals();
    vi.useRealTimers();
    document.body.innerHTML = "";
  });

  it("control: mode 'raw' with raw doc", () => {
    let inst: any;
    const { socket } = openCollab((i) => { inst = i; });
    socket.deliver([MSG_INIT, 0, rawDoc, "raw"]);
    console.log("CONTROL mode=", JSON.stringify(inst?.mode), "text=", JSON.stringify(inst?.view.state.doc.textContent));
    expect(inst?.mode).toBe("raw");
    expect(inst?.view.state.doc.textContent).toBe("hello raw");
  });

  it("mode 123 (number) is adopted verbatim", () => {
    let inst: any;
    const { socket } = openCollab((i) => { inst = i; });
    socket.deliver([MSG_INIT, 0, rawDoc, 123]);
    console.log("NUM mode=", JSON.stringify(inst?.mode), "typeof=", typeof inst?.mode, "text=", JSON.stringify(inst?.view.state.doc.textContent));
  });

  it("mode {} (object) is adopted verbatim", () => {
    let inst: any;
    const { socket } = openCollab((i) => { inst = i; });
    socket.deliver([MSG_INIT, 0, richDoc, { type: "doc" }]);
    console.log("OBJ mode=", JSON.stringify(inst?.mode), "typeof=", typeof inst?.mode, "text=", JSON.stringify(inst?.view.state.doc.textContent));
  });

  it("mode 'bogus' (unknown string) is adopted verbatim", () => {
    let inst: any;
    const { socket } = openCollab((i) => { inst = i; });
    socket.deliver([MSG_INIT, 0, rawDoc, "bogus"]);
    console.log("BOGUS mode=", JSON.stringify(inst?.mode), "text=", JSON.stringify(inst?.view.state.doc.textContent));
  });

  it("mode 0 (falsy non-string) with a RICH doc: falls back to raw schema", () => {
    let inst: any;
    const { socket } = openCollab((i) => { inst = i; });
    socket.deliver([MSG_INIT, 0, richDoc, 0]);
    console.log("ZERO mode=", JSON.stringify(inst?.mode), "text=", JSON.stringify(inst?.view.state.doc.textContent), "doc=", inst?.view.state.doc.toString());
  });

  it("mode 'markdown' control with rich doc works", () => {
    let inst: any;
    const { socket } = openCollab((i) => { inst = i; });
    socket.deliver([MSG_INIT, 0, richDoc, "markdown"]);
    console.log("MD mode=", JSON.stringify(inst?.mode), "text=", JSON.stringify(inst?.view.state.doc.textContent));
    expect(inst?.view.state.doc.textContent).toBe("hello rich");
  });
});
