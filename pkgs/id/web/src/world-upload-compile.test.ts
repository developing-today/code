import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";

/**
 * #7928 regression tests: the world upload/compile socket handlers.
 *
 * Both handlers checked isRecord + string `type` (and upload_ready's
 * chunk_bytes) but then used the frame's message / module_hash /
 * diagnostics unchecked: an object message rendered "[object Object]",
 * an array message "a,b", an object module_hash rendered as a bogus
 * success hash, and a bogus module_installed/compiled frame still
 * cleared the admin token. Present-but-non-string fields now take the
 * handler's invalid-response path; well-formed frames are unchanged.
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
}

class FakeResizeObserver {
  constructor(readonly callback: ResizeObserverCallback) {}
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}

const tick = () => new Promise((resolveTick) => setTimeout(resolveTick, 0));

describe("world upload/compile frame field validation (#7928)", () => {
  const realWebSocket = globalThis.WebSocket;
  const realResizeObserver = globalThis.ResizeObserver;
  let root: HTMLElement;
  const q = <T extends HTMLElement>(selector: string): T => {
    const found = root.querySelector<T>(selector);
    if (!found) throw new Error(`missing ${selector}`);
    return found;
  };
  const statusText = () => q("[data-world-status]").textContent ?? "";
  const adminValue = () => q<HTMLInputElement>("[data-world-admin]").value;
  const lastSocket = () => FakeSocket.all[FakeSocket.all.length - 1];

  const startUpload = async (): Promise<FakeSocket> => {
    q<HTMLInputElement>("[data-world-admin]").value = "secret-token";
    q<HTMLButtonElement>("[data-world-install]").click();
    const socket = lastSocket();
    socket.open();
    await tick();
    return socket;
  };

  const startCompile = async (): Promise<FakeSocket> => {
    q<HTMLInputElement>("[data-world-admin]").value = "secret-token";
    const before = FakeSocket.all.length;
    q<HTMLButtonElement>("[data-world-compile]").click();
    // The compile handler is async: it reads the source files before
    // opening the socket, so wait for the new socket to appear.
    for (let i = 0; i < 20 && FakeSocket.all.length === before; i++) await tick();
    expect(FakeSocket.all.length).toBe(before + 1);
    const socket = lastSocket();
    socket.open();
    await tick();
    return socket;
  };

  beforeAll(async () => {
    vi.stubGlobal("WebSocket", FakeSocket);
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    sessionStorage.clear();
    document.body.innerHTML = worldPageMarkup();
    await import("./world");
    root = document.querySelector<HTMLElement>("[data-world-app]") as HTMLElement;
    expect(root.dataset.initialized).toBe("true");

    const wasm = new File([new Uint8Array([1, 2, 3])], "module.wasm");
    Object.defineProperty(q<HTMLInputElement>("[data-world-module]"), "files", {
      value: [wasm],
      configurable: true,
    });
    const source = new File(["main = 1"], "main.roc");
    Object.defineProperty(q<HTMLInputElement>("[data-world-source]"), "files", {
      value: [source],
      configurable: true,
    });
    q<HTMLInputElement>("[data-world-seed]").value = "1";
  });

  afterAll(() => {
    vi.stubGlobal("WebSocket", realWebSocket);
    vi.stubGlobal("ResizeObserver", realResizeObserver);
    document.body.innerHTML = "";
  });

  it("upload: an object error message takes the invalid-response path", async () => {
    const socket = await startUpload();
    socket.deliver({ type: "error", message: { detail: "boom" } });
    await tick();
    expect(statusText()).toBe("Host returned an invalid upload response");
    expect(socket.readyState).toBe(FakeSocket.CLOSED);
    expect(adminValue()).toBe("secret-token");
  });

  it("upload: an array error message is not rendered as 'a,b'", async () => {
    const socket = await startUpload();
    socket.deliver({ type: "error", message: ["a", "b"] });
    await tick();
    expect(statusText()).toBe("Host returned an invalid upload response");
    expect(socket.readyState).toBe(FakeSocket.CLOSED);
  });

  it("upload: a numeric module_hash is not a success — the admin token survives", async () => {
    const socket = await startUpload();
    socket.deliver({ type: "module_installed", module_hash: 12345 });
    await tick();
    expect(statusText()).toBe("Host returned an invalid upload response");
    expect(socket.readyState).toBe(FakeSocket.CLOSED);
    expect(adminValue()).toBe("secret-token");
  });

  it("upload: a well-formed module_installed behaves as before", async () => {
    const socket = await startUpload();
    socket.deliver({ type: "module_installed", module_hash: "abc123" });
    await tick();
    expect(statusText()).toBe("World program installed: abc123");
    expect(adminValue()).toBe("");
    expect(socket.readyState).toBe(FakeSocket.CLOSED);
  });

  it("upload: a well-formed error message renders as before", async () => {
    const socket = await startUpload();
    socket.deliver({ type: "error", message: "install refused" });
    await tick();
    expect(statusText()).toBe("install refused");
    expect(socket.readyState).toBe(FakeSocket.CLOSED);
  });

  it("compile: an object error message takes the invalid-response path", async () => {
    const socket = await startCompile();
    socket.deliver({ type: "error", message: { detail: "boom" } });
    await tick();
    expect(statusText()).toBe("Host returned an invalid compile response");
    expect(socket.readyState).toBe(FakeSocket.CLOSED);
    expect(adminValue()).toBe("secret-token");
  });

  it("compile: object diagnostics on a compiled frame is not a success", async () => {
    const socket = await startCompile();
    socket.deliver({ type: "compiled", module_hash: "abc123", diagnostics: { lines: 3 } });
    await tick();
    expect(statusText()).toBe("Host returned an invalid compile response");
    expect(socket.readyState).toBe(FakeSocket.CLOSED);
    expect(adminValue()).toBe("secret-token");
  });

  it("compile: an array module_hash is not rendered as a success hash", async () => {
    const socket = await startCompile();
    socket.deliver({ type: "compiled", module_hash: ["a", "b"] });
    await tick();
    expect(statusText()).toBe("Host returned an invalid compile response");
    expect(adminValue()).toBe("secret-token");
  });

  it("compile: a well-formed compiled frame behaves as before", async () => {
    const socket = await startCompile();
    socket.deliver({ type: "compiled", module_hash: "def456", diagnostics: "compiled with warnings" });
    await tick();
    expect(statusText()).toBe("Roc program installed: def456");
    expect(adminValue()).toBe("");
    expect(q("[data-world-log]").textContent).toContain("compiled with warnings");
  });
});
