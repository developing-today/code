import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { clickWorldJoin } from "./world-test-connect";

/**
 * #12648 / #10927 regression tests: browser reconnect continuity.
 *
 * Oracle: resume patches are safe only when the client can prove it still
 * holds the host's exact history. The offline store has no durable journal
 * identity, so after a disconnect the browser asks for an authoritative
 * snapshot and full view instead of offering its old cursor or resume token.
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

class FakeSocket extends EventTarget {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static readonly all: FakeSocket[] = [];

  readyState = FakeSocket.CONNECTING;
  bufferedAmount = 0;
  binaryType = "arraybuffer";
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

  /** Server-initiated drop: the close EVENT arrives asynchronously, as in a browser. */
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

describe("world page reconnect resume (#12648)", () => {
  const realWebSocket = globalThis.WebSocket;
  const realResizeObserver = globalThis.ResizeObserver;
  let root: HTMLElement;
  const q = <T extends HTMLElement>(selector: string): T => {
    const found = root.querySelector<T>(selector);
    if (!found) throw new Error(`missing ${selector}`);
    return found;
  };
  const viewText = () => q("[data-world-view]").textContent ?? "";

  const screenA = {
    kind: "screen",
    version: 1,
    width: 1,
    height: 1,
    cells: [{ symbol: "A", fg: 7, bg: 0, bold: false, columns: 1 }],
  };

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

  it("re-snapshots after a disconnect instead of applying a patch from an unproven history", async () => {
    q<HTMLInputElement>("[data-world-capability]").value = "guest-capability";
    clickWorldJoin(root);
    const first = FakeSocket.all.at(-1) as FakeSocket;
    first.open();
    expect(first.sent("join")).toHaveLength(1);
    first.deliver({
      type: "snapshot",
      snapshot: { world_id: "lobby", current_sequence: 3, participants: [] },
    });
    first.deliver({
      type: "view",
      sequence: 3,
      data: "",
      surface: screenA,
      view_revision: 1,
      resume_token: "resume-1",
    });
    expect(viewText()).toContain("A");

    // Drop and let the backoff reconnect happen on its own.
    first.serverDrop();
    await vi.waitFor(() => expect(FakeSocket.all.length).toBeGreaterThan(1), { timeout: 3000, interval: 50 });
    const resumed = FakeSocket.all.at(-1) as FakeSocket;
    resumed.open();

    // A disconnect invalidates the proof of continuity. The browser
    // requests a fresh snapshot, omits the retained token, and establishes
    // the viewport so the host sends a complete view.
    expect(resumed.sent("join")[0]).toMatchObject({ after: null });
    expect(resumed.sent("join")[0]?.resume_token).toBeUndefined();
    expect(resumed.sent("resize")).toHaveLength(1);

    resumed.deliver({
      type: "snapshot",
      snapshot: { world_id: "lobby", current_sequence: 4, participants: [] },
    });
    resumed.deliver({ type: "view", sequence: 4, data: "authoritative view", view_revision: 2 });
    expect(viewText()).toContain("authoritative view");
  });

  it("drops the resume unit when the viewport actually changes", async () => {
    const current = FakeSocket.all.at(-1) as FakeSocket;
    const pane = q<HTMLElement>("[data-world-view]");
    Object.defineProperty(pane, "clientWidth", { configurable: true, value: 96 });
    Object.defineProperty(pane, "clientHeight", { configurable: true, value: 32 });
    for (const observer of FakeResizeObserver.all) observer.fire();
    // 96 / (16 * 0.6) = 10 columns, 32 / 16 = 2 rows.
    expect(current.sent("resize").at(-1)).toMatchObject({ width: 10, height: 2 });

    current.serverDrop();
    await vi.waitFor(() => expect(FakeSocket.all.length).toBeGreaterThan(2), { timeout: 3000, interval: 50 });
    const rejoined = FakeSocket.all.at(-1) as FakeSocket;
    rejoined.open();
    expect(rejoined.sent("join")[0].resume_token).toBeUndefined();
    // A fresh session still reports its viewport on open.
    expect(rejoined.sent("resize").length).toBeGreaterThan(0);
  });
});
