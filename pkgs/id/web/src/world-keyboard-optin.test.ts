import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, describe, expect, it, vi } from "vitest";
import { clickWorldJoin } from "./world-test-connect";

/**
 * Slice F of #787: the per-connection keyboard opt-in. The page's keydown
 * path must send legacy bytes by default and kitty-disambiguate-v1 bytes
 * when the connection is opened with `?keyboard=kitty`, over the same
 * DOM → WebSocket route the shell contract test exercises. Paste and the
 * manual input form stay on their existing paths in both modes.
 */

const here = dirname(fileURLToPath(import.meta.url));

/** The world page markup, read from the Rust template so the test cannot drift from it. */
function worldPageMarkup(): string {
  const source = readFileSync(resolve(here, "../../crates/id-web/src/templates.rs"), "utf8");
  const start = source.indexOf("pub fn render_world_page");
  const open = source.indexOf('r#"', start);
  const close = source.indexOf('"#;', open);
  if (start < 0 || open < 0 || close < 0) throw new Error("world page template not found in templates.rs");
  return source.slice(open + 3, close);
}

type Entry = { from: "client" | "server"; frame: unknown };

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

const realWebSocket = globalThis.WebSocket;
const realResizeObserver = globalThis.ResizeObserver;

async function bootWorld(search: string): Promise<{ root: HTMLElement; session: FakeSocket }> {
  vi.resetModules();
  vi.stubGlobal("WebSocket", FakeSocket);
  vi.stubGlobal("ResizeObserver", FakeResizeObserver);
  sessionStorage.clear();
  window.history.pushState(null, "", `/world${search}`);
  document.body.innerHTML = worldPageMarkup();
  await import("./world");
  const root = document.querySelector<HTMLElement>("[data-world-app]") as HTMLElement;
  expect(root.dataset.initialized).toBe("true");
  const capability = root.querySelector<HTMLInputElement>("[data-world-capability]");
  if (!capability) throw new Error("missing capability input");
  capability.value = "guest-capability";
  clickWorldJoin(root);
  const session = FakeSocket.all.at(-1) as FakeSocket;
  session.open();
  expect(session.sent("join")).toHaveLength(1);
  return { root, session };
}

function keydown(root: HTMLElement, init: KeyboardEventInit): void {
  const view = root.querySelector("[data-world-view]");
  if (!view) throw new Error("missing world view");
  view.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
}

function inputHexes(session: FakeSocket): unknown[] {
  return session.sent("input").map((frame) => frame.data_hex);
}

afterAll(() => {
  vi.stubGlobal("WebSocket", realWebSocket);
  vi.stubGlobal("ResizeObserver", realResizeObserver);
  document.body.innerHTML = "";
});

describe("keyboard opt-in (#787 Slice F)", () => {
  it("sends legacy bytes by default: bare Escape, chunked legacy forms", async () => {
    const { root, session } = await bootWorld("");
    keydown(root, { key: "Escape" });
    keydown(root, { key: "a" });
    keydown(root, { key: "ArrowUp" });
    // Legacy: Escape is the bare ESC byte — ambiguous with Alt by design
    // of the legacy grammar, and unchanged by this slice.
    expect(inputHexes(session)).toEqual(["1b", "61", "1b5b41"]);
  });

  it("sends kitty-disambiguate-v1 bytes with ?keyboard=kitty", async () => {
    const { root, session } = await bootWorld("?keyboard=kitty");
    keydown(root, { key: "Escape" });
    keydown(root, { key: "i", ctrlKey: true });
    keydown(root, { key: "x", altKey: true });
    keydown(root, { key: "a" });
    // CSI 27 u (Escape, unambiguous), CSI 105;5 u (Ctrl+i, distinct from
    // Tab), CSI 120;3 u (Alt+x, distinct from Escape-then-x), plain text.
    expect(inputHexes(session)).toEqual(["1b5b323775", "1b5b3130353b3575", "1b5b3132303b3375", "61"]);
  });

  it("keeps paste and shortcut exclusions unchanged under the opt-in", async () => {
    const { root, session } = await bootWorld("?keyboard=kitty");
    keydown(root, { key: "s", metaKey: true });
    expect(session.sent("input")).toHaveLength(0);
    const view = root.querySelector("[data-world-view]");
    if (!view) throw new Error("missing world view");
    const paste = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(paste, "clipboardData", { value: { getData: () => "pasted text" } });
    view.dispatchEvent(paste);
    expect(session.sent("paste")).toEqual([{ type: "paste", text: "pasted text" }]);
    expect(session.sent("input")).toHaveLength(0);
  });
});
