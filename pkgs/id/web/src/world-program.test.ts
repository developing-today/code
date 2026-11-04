import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));

/** The program page markup, read from the Rust template so the test cannot drift from it. */
function programPageMarkup(world: string): string {
  const source = readFileSync(resolve(here, "../../src/web/templates.rs"), "utf8");
  const start = source.indexOf("pub fn render_program_page");
  const open = source.indexOf('r#"', start);
  const close = source.indexOf('"#;', open);
  if (start < 0 || open < 0 || close < 0) throw new Error("program page template not found in templates.rs");
  // The server substitutes the escaped world name for __WORLD__ and
  // stamps the socket on the root element; mirror both here.
  return source
    .slice(open + 3, close)
    .replaceAll("__WORLD__", world)
    .replace("data-world-app", 'data-world-app data-world-socket="/ws/world"');
}

type Entry = { from: "client" | "server"; frame: unknown };

/** A WebSocket stand-in that records what the shell sends and lets the test answer. */
class FakeSocket extends EventTarget {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static readonly all: FakeSocket[] = [];

  readyState = FakeSocket.CONNECTING;
  bufferedAmount = 0;
  onclose: ((event: Event) => void) | null = null;
  readonly transcript: Entry[] = [];

  constructor(readonly url: string) {
    super();
    FakeSocket.all.push(this);
  }

  send(data: string): void {
    if (this.readyState !== FakeSocket.OPEN) throw new Error(`send on a socket in state ${this.readyState}`);
    this.transcript.push({ from: "client", frame: JSON.parse(data) });
  }

  close(): void {
    if (this.readyState === FakeSocket.CLOSED) return;
    this.readyState = FakeSocket.CLOSED;
    this.dispatchEvent(new Event("close"));
  }

  open(): void {
    this.readyState = FakeSocket.OPEN;
    this.dispatchEvent(new Event("open"));
  }

  deliver(frame: Record<string, unknown>): void {
    this.transcript.push({ from: "server", frame });
    this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(frame) }));
  }

  sent(type: string): Array<Record<string, unknown>> {
    return this.transcript
      .filter((entry) => entry.from === "client" && (entry.frame as { type?: string }).type === type)
      .map((entry) => entry.frame as Record<string, unknown>);
  }
}

class FakeResizeObserver {
  static readonly all: FakeResizeObserver[] = [];
  constructor(readonly callback: ResizeObserverCallback) {
    FakeResizeObserver.all.push(this);
  }
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
  fire(): void {
    this.callback([], this as unknown as ResizeObserver);
  }
}

describe("program-only surface (#2146 phase 2)", () => {
  const realWebSocket = globalThis.WebSocket;
  const realResizeObserver = globalThis.ResizeObserver;
  let root: HTMLElement;
  const q = <T extends HTMLElement>(selector: string): T => {
    const found = root.querySelector<T>(selector);
    if (!found) throw new Error(`missing ${selector}`);
    return found;
  };

  beforeAll(async () => {
    vi.stubGlobal("WebSocket", FakeSocket);
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    sessionStorage.clear();
    document.body.innerHTML = programPageMarkup("arena");
    await import("./world");
  });

  afterAll(() => {
    vi.stubGlobal("WebSocket", realWebSocket);
    vi.stubGlobal("ResizeObserver", realResizeObserver);
    document.body.innerHTML = "";
  });

  it("initializes without host controls and joins only the pinned world", async () => {
    root = document.querySelector<HTMLElement>("[data-world-app]") as HTMLElement;
    // The presenter initialized even though the page has no host
    // controls (no admin token, invite, install, or host-view hooks).
    expect(root.dataset.initialized).toBe("true");
    expect(root.querySelector("[data-world-admin]")).toBeNull();
    expect(root.querySelector("[data-world-install]")).toBeNull();
    expect(root.querySelector("[data-world-host-view]")).toBeNull();
    expect(root.dataset.worldProgram).toBe("arena");
    expect(q<HTMLInputElement>("[data-world-world]").value).toBe("arena");

    // Join sends the capability to the pinned world, nowhere else.
    q<HTMLInputElement>("[data-world-capability]").value = "guest-capability";
    q<HTMLButtonElement>("[data-world-join]").click();
    const session = FakeSocket.all.at(-1) as FakeSocket;
    expect(session.url).toMatch(/\/ws\/world$/);
    session.open();
    expect(session.sent("join")).toEqual([
      { type: "join", capability: "guest-capability", after: null, world: "arena", screen_patches: true },
    ]);

    // Text rendering through the trusted presenter: guest output
    // arrives as data and lands as view text.
    session.deliver({
      type: "snapshot",
      snapshot: { world_id: "arena", current_sequence: 4, participants: [{ id: 1, display_name: "ada" }], seed: 0 },
    });
    session.deliver({ type: "view", sequence: 4, data: "hello arena", surface: null, view_revision: 1 });
    expect(q("[data-world-view]").textContent).toBe("hello arena");

    // Chat still flows on the guest surface.
    q<HTMLInputElement>("[data-world-chat]").value = "hi all";
    q<HTMLFormElement>("[data-world-chat-form]").dispatchEvent(new Event("submit", { cancelable: true }));
    expect(session.sent("chat")).toEqual([{ type: "chat", text: "hi all" }]);
  });
});
