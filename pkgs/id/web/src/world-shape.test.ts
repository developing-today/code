import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";

/**
 * #7831 regression tests: the world page's HTTP doors validate shape.
 *
 * /api/world/host-view and /api/world/invite used to be shape-trusted —
 * a wrong-shape 200 surfaced a raw TypeError as the host-view status,
 * and the invite path reported fake success while persisting a bogus
 * capability ('undefined' / coerced) to sessionStorage. Both doors now
 * fail closed through the existing error paths. The malformed cases
 * below fail pre-fix.
 */

const here = dirname(fileURLToPath(import.meta.url));

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
    this.sentFrames.push(JSON.parse(data) as Record<string, unknown>);
  }

  close(): void {
    if (this.readyState === FakeSocket.CLOSED) return;
    this.readyState = FakeSocket.CLOSED;
    setTimeout(() => this.dispatchEvent(new Event("close")), 0);
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

const flushAsync = () => new Promise((resolveFlush) => setTimeout(resolveFlush, 20));

describe("world page HTTP response shape validation (#7831)", () => {
  const realWebSocket = globalThis.WebSocket;
  const realResizeObserver = globalThis.ResizeObserver;
  const realFetch = globalThis.fetch;
  let root: HTMLElement;
  const q = <T extends HTMLElement>(selector: string): T => {
    const found = root.querySelector<T>(selector);
    if (!found) throw new Error(`missing ${selector}`);
    return found;
  };
  const status = () => q("[data-world-status]").textContent;
  const storageKey = () => `id-world-capability:${window.location.host}`;
  let payload: unknown = null;

  beforeAll(async () => {
    vi.stubGlobal("WebSocket", FakeSocket);
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(JSON.stringify(payload), { status: 200, headers: { "content-type": "application/json" } }),
      ),
    );
    sessionStorage.clear();
    document.body.innerHTML = worldPageMarkup();
    await import("./world");
    root = document.querySelector<HTMLElement>("[data-world-app]") as HTMLElement;
    expect(root.dataset.initialized).toBe("true");
  });

  afterAll(() => {
    vi.stubGlobal("WebSocket", realWebSocket);
    vi.stubGlobal("ResizeObserver", realResizeObserver);
    vi.stubGlobal("fetch", realFetch);
    document.body.innerHTML = "";
  });

  it.each([
    ["null", null],
    ["empty object", {}],
    ["string", "abc"],
    ["array", []],
    ["records null", { records_sequence: 0, records: null, next_after: null }],
    ["records missing", { records_sequence: 0, next_after: null }],
    ["records_sequence missing", { records: {}, next_after: null }],
    ["next_after wrong type", { records_sequence: 0, records: {}, next_after: 7 }],
  ])("host-view malformed payload (%s) fails closed with no partial render", async (_label, p) => {
    payload = p;
    const recordsBefore = q("[data-world-records]").textContent;
    q<HTMLInputElement>("[data-world-admin]").value = "admin-secret";
    q<HTMLButtonElement>("[data-world-host-view]").click();
    await flushAsync();
    expect(status()).toBe("Host view unavailable");
    expect(q("[data-world-records]").textContent).toBe(recordsBefore);
  });

  it("host-view well-formed payload still renders (control)", async () => {
    payload = {
      snapshot: { world_id: "lobby", current_sequence: 0, active_module_hash: null, participants: [] },
      view: null,
      surface: null,
      records_sequence: 0,
      records: { k: 1 },
      next_after: null,
    };
    q<HTMLInputElement>("[data-world-admin]").value = "admin-secret";
    q<HTMLButtonElement>("[data-world-host-view]").click();
    await flushAsync();
    expect(status()).toBe("Read-only host view");
    expect(q("[data-world-records]").textContent).toContain('"k": 1');
  });

  it.each([
    ["null", null],
    ["empty object", {}],
    ["string", "abc"],
    ["array", []],
    ["numeric capability", { capability: 123, display_name: "ada" }],
    ["empty capability", { capability: "", display_name: "ada" }],
    ["missing display_name", { capability: "cap-x" }],
  ])("invite malformed payload (%s) fails with nothing stored or shown", async (_label, p) => {
    payload = p;
    sessionStorage.clear();
    q<HTMLInputElement>("[data-world-capability]").value = "";
    q<HTMLInputElement>("[data-world-admin]").value = "admin-secret";
    q<HTMLButtonElement>("[data-world-invite]").click();
    await flushAsync();
    expect(status()).toBe("Invite failed");
    expect(q<HTMLInputElement>("[data-world-capability]").value).toBe("");
    expect(sessionStorage.getItem(storageKey())).toBeNull();
    expect(q("[data-world-invite-output]").hidden).toBe(true);
  });

  it("invite well-formed payload still shows the capability (control)", async () => {
    payload = { capability: "cap-123", display_name: "ada" };
    sessionStorage.clear();
    q<HTMLInputElement>("[data-world-admin]").value = "admin-secret";
    q<HTMLButtonElement>("[data-world-invite]").click();
    await flushAsync();
    expect(status()).toContain("Invite created");
    expect(q<HTMLInputElement>("[data-world-capability]").value).toBe("cap-123");
    expect(sessionStorage.getItem(storageKey())).toBe("cap-123");
    expect(q("[data-world-invite-output]").hidden).toBe(false);
  });
});
