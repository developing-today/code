import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { clickWorldJoin } from "./world-test-connect";

/**
 * #10401 regression tests: the world page's snapshot/event validators
 * checked sequence and participant-id fields only for finiteness, so
 * -1, 1.5 and 1e100 passed although they are protocol u64s. An accepted
 * snapshot assigns current_sequence straight to the reconnect cursor
 * and an accepted event raises it via Math.max, poisoning the resume
 * position sent as the join frame's after on reconnect. Oracle: a
 * frame is accepted iff current_sequence / sequence / participant_id /
 * participant id are safe non-negative integers (Number.isSafeInteger
 * && >= 0, mirroring PR #4250's collab INIT version rule); a rejected
 * frame changes no cursor, participant, or log state and surfaces only
 * the invalid-frame status.
 */

const here = dirname(fileURLToPath(import.meta.url));

function worldPageMarkup(): string {
  const source = readFileSync(resolve(here, "../../crates/id-web/src/templates.rs"), "utf8");
  const start = source.indexOf("pub fn render_world_page");
  const open = source.indexOf('r#"', start);
  const close = source.indexOf('"#;', open);
  if (start < 0 || open < 0 || close < 0) throw new Error("world page template not found in templates.rs");
  return source.slice(open + 3, close);
}

class FakeSocket extends EventTarget {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static readonly all: FakeSocket[] = [];
  readyState = FakeSocket.CONNECTING;
  bufferedAmount = 0;
  onclose: ((event: Event) => void) | null = null;
  readonly sentFrames: Array<Record<string, unknown>> = [];
  constructor(readonly url: string) {
    super();
    FakeSocket.all.push(this);
  }
  send(data: string): void {
    if (this.readyState !== FakeSocket.OPEN) throw new Error(`send on a socket in state ${this.readyState}`);
    this.sentFrames.push(JSON.parse(data) as Record<string, unknown>);
  }
  close(): void {
    if (this.readyState === FakeSocket.CLOSED) return;
    this.readyState = FakeSocket.CLOSED;
    setTimeout(() => this.dispatchEvent(new Event("close")), 0);
  }
  serverDrop(): void {
    this.close();
  }
  open(): void {
    this.readyState = FakeSocket.OPEN;
    this.dispatchEvent(new Event("open"));
  }
  deliver(frame: Record<string, unknown>): void {
    this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(frame) }));
  }
  sent(type: string): Array<Record<string, unknown>> {
    return this.sentFrames.filter((frame) => frame.type === type);
  }
}

class FakeResizeObserver {
  constructor(readonly callback: ResizeObserverCallback) {}
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}

const flushAsync = () => new Promise((resolveFlush) => setTimeout(resolveFlush, 10));

describe("world page snapshot/event sequence shapes (#10401)", () => {
  const realWebSocket = globalThis.WebSocket;
  const realResizeObserver = globalThis.ResizeObserver;
  let root: HTMLElement;
  let socket: FakeSocket;
  const q = <T extends HTMLElement>(selector: string): T => {
    const found = root.querySelector<T>(selector);
    if (!found) throw new Error(`missing ${selector}`);
    return found;
  };
  const status = () => q<HTMLElement>("[data-world-status]");
  const participants = () => q<HTMLElement>("[data-world-participants]");
  const log = () => q<HTMLOListElement>("[data-world-log]");

  beforeAll(async () => {
    vi.stubGlobal("WebSocket", FakeSocket);
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    sessionStorage.clear();
    document.body.innerHTML = worldPageMarkup();
    await import("./world");
    root = document.querySelector<HTMLElement>("[data-world-app]") as HTMLElement;
    expect(root.dataset.initialized).toBe("true");
    q<HTMLInputElement>("[data-world-capability]").value = "guest-capability";
    clickWorldJoin(root);
    expect(FakeSocket.all).toHaveLength(1);
    socket = FakeSocket.all[0];
    socket.open();
    socket.deliver({
      type: "snapshot",
      snapshot: { world_id: "lobby", current_sequence: 4, participants: [{ id: 1, display_name: "ada" }] },
    });
    expect(status().textContent).toBe("Connected");
    expect(participants().textContent).toContain("ada");
  });

  afterAll(() => {
    vi.stubGlobal("WebSocket", realWebSocket);
    vi.stubGlobal("ResizeObserver", realResizeObserver);
    document.body.innerHTML = "";
  });

  it("rejects snapshot current_sequence values -1, 1.5 and 1e100 without touching participants", () => {
    for (const bad of [-1, 1.5, 1e100]) {
      socket.deliver({
        type: "snapshot",
        snapshot: { world_id: "lobby", current_sequence: bad, participants: [{ id: 2, display_name: "mallory" }] },
      });
      expect(status().textContent).toBe("Received an invalid world frame");
      expect(status().dataset.state).toBe("error");
      expect(participants().textContent).toContain("ada");
      expect(participants().textContent).not.toContain("mallory");
    }
  });

  it("rejects a snapshot whose participant id is fractional", () => {
    socket.deliver({
      type: "snapshot",
      snapshot: { world_id: "lobby", current_sequence: 5, participants: [{ id: 1.5, display_name: "mallory" }] },
    });
    expect(status().textContent).toBe("Received an invalid world frame");
    expect(participants().textContent).toContain("ada");
    expect(participants().textContent).not.toContain("mallory");
  });

  it("rejects event sequence values -1, 1.5 and 1e100 and a fractional participant_id without logging", () => {
    const linesBefore = log().childElementCount;
    const malformed: Array<Record<string, unknown>> = [
      { sequence: -1, participant_id: 1, kind: { kind: "chat", data: "neg" } },
      { sequence: 1.5, participant_id: 1, kind: { kind: "chat", data: "frac" } },
      { sequence: 1e100, participant_id: 1, kind: { kind: "chat", data: "huge" } },
      { sequence: 5, participant_id: 1.5, kind: { kind: "chat", data: "who" } },
    ];
    for (const event of malformed) {
      socket.deliver({ type: "event", event });
      expect(status().textContent).toBe("Received an invalid world frame");
    }
    expect(log().childElementCount).toBe(linesBefore + malformed.length);
    expect(log().textContent).not.toContain("neg");
    expect(log().textContent).not.toContain("frac");
    expect(log().textContent).not.toContain("huge");
    expect(log().textContent).not.toContain("who");
    expect(log().textContent).not.toContain("player 1.5");
  });

  it("control: valid frames still apply, including participant id 0 and a safe-integer sequence", () => {
    socket.deliver({
      type: "event",
      event: { sequence: 5, participant_id: 1, kind: { kind: "chat", data: "hello" } },
    });
    expect(log().textContent).toContain("ada: hello");
    socket.deliver({
      type: "snapshot",
      snapshot: { world_id: "lobby", current_sequence: 6, participants: [{ id: 0, display_name: "zero" }] },
    });
    expect(status().textContent).toBe("Connected");
    expect(participants().textContent).toContain("zero");
  });

  it("reconnect join still sends the pre-malformed-frame cursor as after", async () => {
    const countBefore = FakeSocket.all.length;
    socket.serverDrop();
    await flushAsync();
    await vi.waitFor(() => expect(FakeSocket.all).toHaveLength(countBefore + 1), { timeout: 3000, interval: 50 });
    const reconnected = FakeSocket.all.at(-1) as FakeSocket;
    reconnected.open();
    expect(reconnected.sent("join")).toHaveLength(1);
    expect(reconnected.sent("join")[0].after).toBe(6);
  });
});
