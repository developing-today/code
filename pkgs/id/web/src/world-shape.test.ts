import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";

/**
 * #7831 regression tests: the world page's two HTTP fetch boundaries
 * (host view, invite) must validate payload shape before use.
 *
 * Oracle: the server contracts in pkgs/id/src/web/world_ws.rs —
 * HostViewResponse requires numeric `records_sequence`, object
 * `records`, and `next_after` string-or-null; the invite response
 * requires a non-empty string `capability` and a string
 * `display_name`. A wrong-shape 200 must fail closed through the
 * handlers' own error vocabulary ("Host view unavailable" /
 * "Invite failed"), never leak a raw TypeError as the status, and
 * never persist a bogus capability to sessionStorage.
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

  constructor(readonly url: string) {
    super();
    FakeSocket.all.push(this);
  }

  send(_data: string): void {}
  close(): void {
    this.readyState = FakeSocket.CLOSED;
  }
}

class FakeResizeObserver {
  constructor(readonly callback: ResizeObserverCallback) {}
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}

const flushAsync = () => new Promise((resolveFlush) => setTimeout(resolveFlush, 10));

const jsonResponse = (payload: unknown) =>
  new Response(JSON.stringify(payload), { status: 200, headers: { "content-type": "application/json" } });

const wellFormedHostView = {
  snapshot: { world_id: "lobby", current_sequence: 0, active_module_hash: null, participants: [] },
  view: null,
  surface: null,
  records_sequence: 7,
  records: { greeting: "hello" },
  next_after: null,
};

describe("world page HTTP shape validation (#7831)", () => {
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

  const stubFetchPayload = (payload: unknown) => {
    vi.stubGlobal("fetch", vi.fn(async () => jsonResponse(payload)));
  };

  beforeAll(async () => {
    vi.stubGlobal("WebSocket", FakeSocket);
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    sessionStorage.clear();
    document.body.innerHTML = worldPageMarkup();
    await import("./world");
    root = document.querySelector<HTMLElement>("[data-world-app]") as HTMLElement;
    expect(root.dataset.initialized).toBe("true");
    q<HTMLInputElement>("[data-world-admin]").value = "admin-secret";
  });

  afterAll(() => {
    vi.stubGlobal("WebSocket", realWebSocket);
    vi.stubGlobal("ResizeObserver", realResizeObserver);
    vi.stubGlobal("fetch", realFetch);
    document.body.innerHTML = "";
  });

  it.each([
    ["null body", null],
    ["empty object", {}],
    ["string body", "abc"],
    ["array body", []],
    ["records null", { records_sequence: 0, records: null, next_after: null }],
    ["records missing", { records_sequence: 0, next_after: null }],
    ["records_sequence wrong type", { records_sequence: "7", records: {}, next_after: null }],
    ["next_after wrong type", { records_sequence: 7, records: {}, next_after: 42 }],
  ])("host view: %s surfaces as 'Host view unavailable', never a raw TypeError", async (_name, payload) => {
    stubFetchPayload(payload);
    q<HTMLButtonElement>("[data-world-host-view]").click();
    await vi.waitFor(() => expect(status()).toBe("Host view unavailable"), { timeout: 15000, interval: 50 });
    expect(status()).not.toMatch(/Cannot (read|convert)/);
    await flushAsync();
  }, 30_000);

  it("host view: a well-formed payload still renders records and the good status", async () => {
    stubFetchPayload(wellFormedHostView);
    q<HTMLButtonElement>("[data-world-host-view]").click();
    await vi.waitFor(() => expect(status()).toBe("Read-only host view"), { timeout: 15000, interval: 50 });
    expect(q("[data-world-records]").textContent).toContain("greeting");
  }, 30_000);

  it.each([
    ["null body", null],
    ["empty object", {}],
    ["string body", "abc"],
    ["array body", []],
    ["capability wrong type", { capability: 123, display_name: "ada" }],
    ["capability empty", { capability: "", display_name: "ada" }],
    ["capability missing display_name", { capability: "cap-x" }],
    ["display_name wrong type", { capability: "cap-x", display_name: 7 }],
  ])("invite: %s fails without persisting anything", async (_name, payload) => {
    sessionStorage.clear();
    q<HTMLInputElement>("[data-world-capability]").value = "";
    q<HTMLElement>("[data-world-invite-output]").hidden = true;
    q<HTMLInputElement>("[data-world-admin]").value = "admin-secret";
    stubFetchPayload(payload);
    q<HTMLButtonElement>("[data-world-invite]").click();
    await vi.waitFor(() => expect(status()).toBe("Invite failed"), { timeout: 15000, interval: 50 });
    expect(q<HTMLInputElement>("[data-world-capability]").value).toBe("");
    expect(q<HTMLElement>("[data-world-invite-output]").hidden).toBe(true);
    expect(sessionStorage.getItem(storageKey())).toBeNull();
    await flushAsync();
  }, 30_000);

  it("invite: a well-formed payload still stores the capability and reports success", async () => {
    sessionStorage.clear();
    q<HTMLInputElement>("[data-world-admin]").value = "admin-secret";
    stubFetchPayload({ capability: "cap-123", display_name: "ada" });
    q<HTMLButtonElement>("[data-world-invite]").click();
    await vi.waitFor(() => expect(status()).toContain("Invite created"), { timeout: 15000, interval: 50 });
    expect(q<HTMLInputElement>("[data-world-capability]").value).toBe("cap-123");
    expect(sessionStorage.getItem(storageKey())).toBe("cap-123");
    expect(q<HTMLElement>("[data-world-invite-output]").hidden).toBe(false);
  }, 30_000);
});
