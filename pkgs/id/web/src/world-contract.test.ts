import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { Conformance, checkClientFrame, checkServerFrame, type ShellContract } from "./shell-contract";

const here = dirname(fileURLToPath(import.meta.url));
const contract = JSON.parse(
  readFileSync(resolve(here, "../../contract/shell-contract.v1.json"), "utf8"),
) as ShellContract;

/** The world page markup, read from the Rust template so the test cannot drift from it. */
function worldPageMarkup(): string {
  const source = readFileSync(resolve(here, "../../src/web/templates.rs"), "utf8");
  const start = source.indexOf("pub fn render_world_page");
  const open = source.indexOf('r#"', start);
  const close = source.indexOf('"#;', open);
  if (start < 0 || open < 0 || close < 0) throw new Error("world page template not found in templates.rs");
  return source.slice(open + 3, close);
}

describe("shell contract validator", () => {
  const cap = "cap";

  it("accepts the frames a joined shell sends", () => {
    expect(checkClientFrame(contract, { type: "join", capability: cap, after: null, world: null }, "opening")).toEqual({
      ok: true,
      type: "join",
    });
    expect(checkClientFrame(contract, { type: "resize", width: 500, height: 200 }, "joined").ok).toBe(true);
    expect(checkClientFrame(contract, { type: "input", data_hex: "1b5b41" }, "joined").ok).toBe(true);
    expect(checkClientFrame(contract, { type: "records", capability: cap, after: "k" }, "joined").ok).toBe(true);
    expect(checkServerFrame(contract, { type: "view", sequence: 1, data: "x", surface: null }).ok).toBe(true);
  });

  it.each([
    ["an unknown type", { type: "teleport" }, "joined", "is not a client frame"],
    ["a missing field", { type: "chat" }, "joined", "needs field `text`"],
    ["an unknown field", { type: "chat", text: "hi", loud: true }, "joined", "has no field `loud`"],
    ["a resize wider than 500", { type: "resize", width: 501, height: 10 }, "joined", "must be in 1..=500"],
    ["non-hex input", { type: "input", data_hex: "zz" }, "joined", "even number of hex digits"],
    ["odd-length hex input", { type: "input", data_hex: "abc" }, "joined", "even number of hex digits"],
    [
      "`uses` in attenuate",
      { type: "attenuate", name: "n", scopes: ["chat"], extra: { uses: 3 } },
      "joined",
      "may not carry `uses`",
    ],
    ["chat before a join", { type: "chat", text: "hi" }, "opening", "may not be sent in the opening phase"],
    ["a non-string routing field", { type: "chat", text: "hi", world: 3 }, "joined", "routing field `world`"],
    ["a paste over 4096 bytes", { type: "paste", text: "é".repeat(2049) }, "joined", "more than 4096"],
    [
      "a role outside the enum",
      { type: "invite", admin_token: "a", display_name: "d", role: "god" },
      "opening",
      "one of",
    ],
  ] as const)("rejects %s", (_name, frame, phase, error) => {
    const check = checkClientFrame(contract, frame, phase);
    expect(check.ok).toBe(false);
    if (!check.ok) expect(check.error).toContain(error);
  });

  it("accepts u64 values at and above 2^53, like the Rust checker (#1441)", () => {
    const seed = (value: number) =>
      checkClientFrame(contract, { type: "install_begin", admin_token: "a", total_bytes: 1, seed: value }, "opening");
    expect(seed(2 ** 53).ok).toBe(true);
    // u64::MAX as it arrives from wire JSON: already rounded by the parser.
    expect(seed(JSON.parse("18446744073709551615") as number).ok).toBe(true);
    expect(checkServerFrame(contract, { type: "view", sequence: 2 ** 53, data: "x", surface: null }).ok).toBe(true);
    expect(checkServerFrame(contract, { type: "records", sequence: 2 ** 53, records: {} }).ok).toBe(true);
  });

  it("rejects u64 values provably above u64::MAX, like the Rust checker (#1441)", () => {
    const seed = (value: number) =>
      checkClientFrame(contract, { type: "install_begin", admin_token: "a", total_bytes: 1, seed: value }, "opening");
    // 2^64 is the rounded wire form of u64::MAX itself: ambiguous, accepted.
    expect(seed(2 ** 64).ok).toBe(true);
    const over64 = seed(2 ** 65);
    expect(over64.ok).toBe(false);
    if (!over64.ok) expect(over64.error).toContain("18446744073709551615");
    expect(seed(1e100).ok).toBe(false);
    expect(checkServerFrame(contract, { type: "view", sequence: 2 ** 65, data: "x", surface: null }).ok).toBe(false);
  });

  it("still enforces the u64 bounds JS can check exactly (#1441)", () => {
    const begin = (patch: Record<string, unknown>) =>
      checkClientFrame(
        contract,
        { type: "install_begin", admin_token: "a", total_bytes: 1, seed: 1, ...patch },
        "opening",
      );
    expect(begin({ seed: -1 }).ok).toBe(false);
    expect(begin({ seed: 1.5 }).ok).toBe(false);
    expect(begin({ total_bytes: 0 }).ok).toBe(false);
    const over = begin({ total_bytes: 16777217 });
    expect(over.ok).toBe(false);
    if (!over.ok) {
      expect(over.error).toContain("16777216");
      expect(over.error).not.toContain("9007199254740991");
    }
  });

  it("rejects a seed leaked at any depth of a server frame", () => {
    const leak = checkServerFrame(contract, { type: "event", event: { kind: { program: { seed: 7 } } } });
    expect(leak).toEqual({ ok: false, error: expect.stringContaining("leaks `seed` (7)") });
    expect(checkServerFrame(contract, { type: "event", event: { kind: { seed: 0 } } }).ok).toBe(true);
    expect(checkServerFrame(contract, { type: "records", sequence: 1, records: {}, extra: 1 }).ok).toBe(false);
  });

  it("follows session phases", () => {
    const query = new Conformance(contract);
    query.client({ type: "info", capability: cap });
    expect(query.phase).toBe("query");
    query.client({ type: "join", capability: cap });
    expect(query.phase).toBe("joined");
    query.client({ type: "chat", text: "hi" });
    expect(query.violations).toEqual([]);

    const closed = new Conformance(contract);
    closed.client({ type: "caps", capability: cap });
    expect(closed.phase).toBe("closed");
    closed.client({ type: "caps", capability: cap });
    expect(closed.violations).toEqual([{ index: 1, side: "client", message: "the host already ended this session" }]);

    const install = new Conformance(contract);
    install.client({ type: "install_begin", admin_token: "a", total_bytes: 1, seed: 1 });
    expect(install.phase).toBe("install");
    install.client({ type: "install_chunk", offset: 0, data_hex: "00" });
    install.client({ type: "install_end" });
    expect(install.phase).toBe("closed");
    expect(install.violations).toEqual([]);

    const refused = new Conformance(contract);
    refused.client({ type: "records", capability: cap });
    refused.server({ type: "error", message: "no" });
    expect(refused.phase).toBe("closed");
    refused.client({ type: "join", capability: cap });
    expect(refused.violations).toHaveLength(1);
  });
});

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

function setFiles(input: HTMLInputElement, files: File[]): void {
  Object.defineProperty(input, "files", { configurable: true, value: files });
}

describe("web shell conformance", () => {
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
    document.body.innerHTML = worldPageMarkup();
    await import("./world");
  });

  afterAll(() => {
    vi.stubGlobal("WebSocket", realWebSocket);
    vi.stubGlobal("ResizeObserver", realResizeObserver);
    document.body.innerHTML = "";
  });

  it("sends only contract frames, in the right phases, across every flow", async () => {
    root = document.querySelector<HTMLElement>("[data-world-app]") as HTMLElement;
    expect(root.dataset.initialized).toBe("true");

    // Join.
    q<HTMLInputElement>("[data-world-capability]").value = "guest-capability";
    q<HTMLButtonElement>("[data-world-join]").click();
    const session = FakeSocket.all.at(-1) as FakeSocket;
    expect(session.url).toMatch(/\/ws\/world$/);
    session.open();
    expect(session.sent("join")).toHaveLength(1);
    expect(session.sent("resize")).toHaveLength(1);

    // Snapshot and view; the snapshot asks for records, which page.
    session.deliver({
      type: "snapshot",
      snapshot: { world_id: "lobby", current_sequence: 4, participants: [{ id: 1, display_name: "ada" }], seed: 0 },
    });
    session.deliver({ type: "view", sequence: 4, data: "hello", surface: null });
    expect(q("[data-world-view]").textContent).toBe("hello");
    expect(session.sent("records")).toEqual([{ type: "records", capability: "guest-capability" }]);
    session.deliver({ type: "records", sequence: 4, records: { a: 1 }, next_after: "a" });
    expect(session.sent("records").at(-1)).toEqual({ type: "records", capability: "guest-capability", after: "a" });
    session.deliver({ type: "records", sequence: 4, records: { b: 2 }, next_after: null });
    expect(JSON.parse(q("[data-world-records]").textContent ?? "")).toEqual({ a: 1, b: 2 });

    // Resize, keys, paste, chat, input line, records refresh.
    for (const observer of FakeResizeObserver.all) observer.fire();
    const view = q("[data-world-view]");
    view.dispatchEvent(new KeyboardEvent("keydown", { key: "a", bubbles: true, cancelable: true }));
    view.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true, cancelable: true }));
    const paste = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(paste, "clipboardData", { value: { getData: () => "pasted text" } });
    view.dispatchEvent(paste);
    q<HTMLInputElement>("[data-world-chat]").value = "hi all";
    q<HTMLFormElement>("[data-world-chat-form]").dispatchEvent(new Event("submit", { cancelable: true }));
    q<HTMLFormElement>("[data-world-input-form]").dispatchEvent(new Event("submit", { cancelable: true }));
    session.deliver({
      type: "event",
      event: { sequence: 5, participant_id: 1, kind: { kind: "chat", data: "hi all" } },
    });
    session.deliver({
      type: "event",
      event: { sequence: 6, participant_id: 1, kind: { kind: "input", data: [97] } },
    });
    session.deliver({ type: "records", sequence: 6, records: {}, next_after: null });
    q<HTMLButtonElement>("[data-world-records-refresh]").click();
    session.deliver({ type: "records", sequence: 6, records: {} });

    expect(session.sent("input").map((frame) => frame.data_hex)).toEqual(["61", "1b5b41", "696e63"]);
    expect(session.sent("paste")).toEqual([{ type: "paste", text: "pasted text" }]);
    expect(session.sent("chat")).toEqual([{ type: "chat", text: "hi all" }]);

    // Module install over its own session.
    q<HTMLInputElement>("[data-world-admin]").value = "admin-secret";
    setFiles(q<HTMLInputElement>("[data-world-module]"), [
      new File([new Uint8Array(9000).fill(7)], "game.wasm", { type: "application/wasm" }),
    ]);
    q<HTMLButtonElement>("[data-world-install]").click();
    const upload = FakeSocket.all.at(-1) as FakeSocket;
    expect(upload).not.toBe(session);
    upload.open();
    upload.deliver({ type: "upload_ready", chunk_bytes: 6144 });
    await vi.waitFor(() => expect(upload.sent("install_end")).toHaveLength(1));
    expect(upload.sent("install_chunk").map((frame) => frame.offset)).toEqual([0, 6144]);
    upload.deliver({ type: "module_installed", module_hash: "abc", sequence: 7 });
    expect(upload.readyState).toBe(FakeSocket.CLOSED);

    // Roc compile over its own session.
    q<HTMLInputElement>("[data-world-admin]").value = "admin-secret";
    setFiles(q<HTMLInputElement>("[data-world-source]"), [
      new File(["app [main!] {}"], "main.roc"),
      new File(["module []"], "Util.roc"),
    ]);
    q<HTMLButtonElement>("[data-world-compile]").click();
    await vi.waitFor(() => expect(FakeSocket.all.at(-1)).not.toBe(upload));
    const compiler = FakeSocket.all.at(-1) as FakeSocket;
    compiler.open();
    compiler.deliver({ type: "compiled", module_hash: "def", sequence: 8, diagnostics: "" });
    expect(compiler.readyState).toBe(FakeSocket.CLOSED);

    // Every recorded session, checked against the contract in order.
    const exercised = new Set<string>();
    const report: string[] = [];
    for (const [number, socket] of FakeSocket.all.entries()) {
      const check = new Conformance(contract);
      for (const entry of socket.transcript) {
        if (entry.from === "client") {
          check.client(entry.frame);
          exercised.add((entry.frame as { type: string }).type);
        } else {
          check.server(entry.frame);
        }
      }
      for (const violation of check.violations) {
        report.push(`socket ${number} frame ${violation.index} (${violation.side}): ${violation.message}`);
      }
    }
    expect(report).toEqual([]);
    expect([...exercised].sort()).toEqual(
      [
        "chat",
        "compile",
        "input",
        "install_begin",
        "install_chunk",
        "install_end",
        "join",
        "paste",
        "records",
        "resize",
      ].sort(),
    );
  });
});
