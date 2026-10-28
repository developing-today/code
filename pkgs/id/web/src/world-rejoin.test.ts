import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";

/**
 * #1461 regression tests: the world page's rejoin path.
 *
 * A real browser dispatches a socket's `close` event ASYNCHRONOUSLY,
 * after the click handler that called `close()` has finished. The fake
 * below reproduces that timing - a synchronous dispatch is exactly
 * what hid this bug from the existing contract tests.
 */

const here = dirname(fileURLToPath(import.meta.url));

/** The world page markup, read from the Rust template so the test cannot drift from it. */
function worldPageMarkup(): string {
  const source = readFileSync(resolve(here, "../../src/web/templates.rs"), "utf8");
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

  /** Client-initiated close: the close EVENT arrives asynchronously, as in a browser. */
  close(): void {
    if (this.readyState === FakeSocket.CLOSED) return;
    this.readyState = FakeSocket.CLOSED;
    setTimeout(() => this.dispatchEvent(new Event("close")), 0);
  }

  /** Server-initiated drop: also asynchronous. */
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
const sleep = (ms: number) => new Promise((resolveSleep) => setTimeout(resolveSleep, ms));

describe("world page rejoin (ghost WebSocket, #1461)", () => {
  const realWebSocket = globalThis.WebSocket;
  const realResizeObserver = globalThis.ResizeObserver;
  let root: HTMLElement;
  const q = <T extends HTMLElement>(selector: string): T => {
    const found = root.querySelector<T>(selector);
    if (!found) throw new Error(`missing ${selector}`);
    return found;
  };
  const status = () => q("[data-world-status]").textContent;

  beforeAll(async () => {
    vi.stubGlobal("WebSocket", FakeSocket);
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    sessionStorage.clear();
    document.body.innerHTML = worldPageMarkup();
    await import("./world");
    root = document.querySelector<HTMLElement>("[data-world-app]") as HTMLElement;
    expect(root.dataset.initialized).toBe("true");
  });

  afterAll(() => {
    vi.stubGlobal("WebSocket", realWebSocket);
    vi.stubGlobal("ResizeObserver", realResizeObserver);
    document.body.innerHTML = "";
  });

  it("rejoin retires the old socket: its async close spawns no ghost connection", async () => {
    q<HTMLInputElement>("[data-world-capability]").value = "guest-capability";
    q<HTMLButtonElement>("[data-world-join]").click();
    expect(FakeSocket.all).toHaveLength(1);
    const first = FakeSocket.all[0];
    first.open();
    expect(first.sent("join")).toHaveLength(1);

    // Rejoin. The click retires socket 1, creates socket 2; socket 1's
    // close event lands afterwards, asynchronously.
    q<HTMLButtonElement>("[data-world-join]").click();
    expect(FakeSocket.all).toHaveLength(2);
    const second = FakeSocket.all[1];
    await flushAsync();

    // The retired socket's close must not overwrite the status line...
    expect(status()).toBe("Connecting…");
    second.open();
    // ...and the current socket joins exactly once, on itself.
    expect(second.sent("join")).toHaveLength(1);
    expect(first.sent("join")).toHaveLength(1);

    second.deliver({
      type: "snapshot",
      snapshot: { world_id: "lobby", current_sequence: 4, participants: [{ id: 1, display_name: "ada" }] },
    });
    expect(status()).toBe("Connected");

    // Frames arriving on the retired socket are ignored, not duplicated.
    const log = q("[data-world-log]");
    const linesBefore = log.childElementCount;
    first.deliver({
      type: "event",
      event: { sequence: 5, participant_id: 1, kind: { kind: "chat", data: "ghost echo" } },
    });
    expect(log.childElementCount).toBe(linesBefore);
    second.deliver({
      type: "event",
      event: { sequence: 5, participant_id: 1, kind: { kind: "chat", data: "real line" } },
    });
    expect(log.childElementCount).toBe(linesBefore + 1);

    // Long past the first reconnect backoff (500ms): still exactly two
    // sockets created - the ghost would have been a third.
    await sleep(700);
    expect(FakeSocket.all).toHaveLength(2);
  });

  it("a genuine unexpected drop still auto-reconnects with backoff", async () => {
    const current = FakeSocket.all.at(-1) as FakeSocket;
    current.serverDrop();
    await flushAsync();
    expect(status()).toBe("Disconnected; retrying…");

    await vi.waitFor(() => expect(FakeSocket.all).toHaveLength(3), { timeout: 3000, interval: 50 });
    const reconnected = FakeSocket.all[2];
    reconnected.open();
    expect(reconnected.sent("join")).toHaveLength(1);
  });
});
