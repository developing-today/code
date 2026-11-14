import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));

/** The program page markup, read from the Rust template so the test cannot drift from it. */
function programPageMarkup(world: string): string {
  const source = readFileSync(resolve(here, "../../crates/id-web/src/templates.rs"), "utf8");
  const start = source.indexOf("pub fn render_program_page");
  const open = source.indexOf('r#"', start);
  const close = source.indexOf('"#;', open);
  if (start < 0 || open < 0 || close < 0) throw new Error("program page template not found in templates.rs");
  return source
    .slice(open + 3, close)
    .replaceAll("__WORLD__", world)
    .replace("data-world-app", 'data-world-app data-world-socket="/ws/world"');
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
  constructor(readonly url: string) {
    super();
    FakeSocket.all.push(this);
  }
  send(): void {}
  close(): void {
    this.readyState = FakeSocket.CLOSED;
    this.dispatchEvent(new Event("close"));
  }
}

class FakeResizeObserver {
  constructor(readonly callback: ResizeObserverCallback) {}
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}

describe("program surface entry point (#2146 phase 2)", () => {
  const realWebSocket = globalThis.WebSocket;
  const realResizeObserver = globalThis.ResizeObserver;
  const fetchSpy = vi.fn(async (_input: unknown) => new Response("{}", { status: 200 }));

  beforeAll(async () => {
    vi.stubGlobal("WebSocket", FakeSocket);
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    vi.stubGlobal("fetch", fetchSpy);
    sessionStorage.clear();
    localStorage.clear();
    document.body.innerHTML = programPageMarkup("arena");
    await import("./main");
    // Let init() settle.
    await new Promise((resolve) => setTimeout(resolve, 50));
  });

  afterAll(() => {
    vi.stubGlobal("WebSocket", realWebSocket);
    vi.stubGlobal("ResizeObserver", realResizeObserver);
    vi.unstubAllGlobals();
    document.body.innerHTML = "";
  });

  it("does not touch Explorer identity or tags endpoints", () => {
    const fetched = fetchSpy.mock.calls.map((call) => String(call[0]));
    expect(fetched.filter((url) => url.includes("/api/identity"))).toEqual([]);
    expect(FakeSocket.all.filter((socket) => socket.url.includes("/ws/tags"))).toEqual([]);
    // The world presenter still initialized on the same page.
    const root = document.querySelector<HTMLElement>("[data-world-app]");
    expect(root?.dataset.initialized).toBe("true");
  });
});
