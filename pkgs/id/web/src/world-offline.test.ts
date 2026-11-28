import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import {
  loadWorldSnapshot,
  parseStoredWorldSnapshot,
  saveWorldSnapshot,
  worldSnapshotStoreKey,
  type StoredWorldSnapshot,
} from "./world-snapshot-store";
import { clickWorldJoin } from "./world-test-connect";

/**
 * #10927 (shells v1 slice 1) behavioral tests: the browser shell's
 * offline read model. Oracle: the design record
 * (doc/2026-11-10T20-40-00Z_design_more_shells_v1) — the shell keeps
 * a local snapshot (last applied journal sequence + rendered state)
 * sufficient to READ the world fully offline; offline the shell is a
 * reader, not a writer; on reconnect it presents its last applied
 * sequence. A snapshot store entry that fails the same shape
 * discipline as the frame validators (safe non-negative integer
 * sequences/ids, string fields) is ignored, never rendered.
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

class FakeServiceWorkerContainer extends EventTarget {
  notify(data: unknown): void {
    this.dispatchEvent(new MessageEvent("message", { data }));
  }
}

class MemoryStorage {
  private readonly map = new Map<string, string>();
  getItem(key: string): string | null {
    return this.map.get(key) ?? null;
  }
  setItem(key: string, value: string): void {
    this.map.set(key, value);
  }
  removeItem(key: string): void {
    this.map.delete(key);
  }
}

const sampleSnapshot: StoredWorldSnapshot = {
  world_id: "lobby",
  sequence: 5,
  participants: [{ id: 1, display_name: "ada" }],
  view: { kind: "text", text: "arena view" },
  view_revision: null,
  records: { note: "hello" },
};

describe("world snapshot store (#10927)", () => {
  it("keys entries per host and world, with the default world distinct", () => {
    expect(worldSnapshotStoreKey("example.test", null)).toBe("id-world-snapshot:example.test:");
    expect(worldSnapshotStoreKey("example.test", "arena")).toBe("id-world-snapshot:example.test:arena");
  });

  it("round-trips a stored snapshot", () => {
    const storage = new MemoryStorage() as unknown as Storage;
    saveWorldSnapshot(storage, "example.test", null, sampleSnapshot);
    expect(loadWorldSnapshot(storage, "example.test", null)).toEqual(sampleSnapshot);
    expect(loadWorldSnapshot(storage, "example.test", "arena")).toBeNull();
    expect(loadWorldSnapshot(storage, "other.test", null)).toBeNull();
  });

  it("rejects entries that fail the frame shape discipline", () => {
    expect(parseStoredWorldSnapshot(null)).toBeNull();
    expect(parseStoredWorldSnapshot("not json")).toBeNull();
    expect(parseStoredWorldSnapshot(JSON.stringify({ ...sampleSnapshot, sequence: -1 }))).toBeNull();
    expect(parseStoredWorldSnapshot(JSON.stringify({ ...sampleSnapshot, sequence: 1.5 }))).toBeNull();
    expect(
      parseStoredWorldSnapshot(JSON.stringify({ ...sampleSnapshot, participants: [{ id: "1", display_name: "ada" }] })),
    ).toBeNull();
    expect(
      parseStoredWorldSnapshot(JSON.stringify({ ...sampleSnapshot, view: { kind: "text", text: 42 } })),
    ).toBeNull();
    expect(parseStoredWorldSnapshot(JSON.stringify({ ...sampleSnapshot, records: [1, 2] }))).toBeNull();
  });
});

describe("world page offline read model (#10927)", () => {
  const realWebSocket = globalThis.WebSocket;
  const realResizeObserver = globalThis.ResizeObserver;
  const realServiceWorker = Object.getOwnPropertyDescriptor(navigator, "serviceWorker");
  let serviceWorker = new FakeServiceWorkerContainer();
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
    Object.defineProperty(navigator, "serviceWorker", { configurable: true, value: serviceWorker });
    sessionStorage.clear();
    localStorage.clear();
    document.body.innerHTML = worldPageMarkup();
    await import("./world");
    root = document.querySelector<HTMLElement>("[data-world-app]") as HTMLElement;
    expect(root.dataset.initialized).toBe("true");
  });

  afterAll(() => {
    vi.stubGlobal("WebSocket", realWebSocket);
    vi.stubGlobal("ResizeObserver", realResizeObserver);
    if (realServiceWorker) Object.defineProperty(navigator, "serviceWorker", realServiceWorker);
    else Reflect.deleteProperty(navigator, "serviceWorker");
    document.body.innerHTML = "";
  });

  it("persists the snapshot store as frames arrive", () => {
    q<HTMLInputElement>("[data-world-capability]").value = "guest-capability";
    clickWorldJoin(root);
    expect(FakeSocket.all).toHaveLength(1);
    const socket = FakeSocket.all[0];
    socket.open();
    expect(socket.sent("join")[0]?.after).toBeNull();

    socket.deliver({
      type: "snapshot",
      snapshot: { world_id: "lobby", current_sequence: 4, participants: [{ id: 1, display_name: "ada" }] },
    });
    expect(status()).toBe("Connected");
    socket.deliver({ type: "view", data: "arena view" });
    expect(socket.sent("records")).toHaveLength(1);
    socket.deliver({ type: "records", records: { note: "hello" }, next_after: null });
    socket.deliver({
      type: "event",
      event: { sequence: 5, participant_id: 1, kind: { kind: "chat", data: "hi" } },
    });

    const raw = localStorage.getItem(worldSnapshotStoreKey(window.location.host, null));
    expect(raw).not.toBeNull();
    const stored = parseStoredWorldSnapshot(raw);
    expect(stored?.world_id).toBe("lobby");
    expect(stored?.sequence).toBe(5);
    expect(stored?.participants).toEqual([{ id: 1, display_name: "ada" }]);
    expect(stored?.view).toEqual({ kind: "text", text: "arena view" });
    expect(stored?.records).toEqual({ note: "hello" });
  });

  it("after a reload, renders the stored snapshot offline and resumes from its sequence", async () => {
    // Reload: fresh module + fresh DOM, same localStorage. No
    // capability in session storage, so nothing auto-connects: the
    // page is offline and must read from the store alone.
    vi.resetModules();
    FakeSocket.all.length = 0;
    serviceWorker = new FakeServiceWorkerContainer();
    Object.defineProperty(navigator, "serviceWorker", { configurable: true, value: serviceWorker });
    sessionStorage.clear();
    document.body.innerHTML = worldPageMarkup();
    await import("./world");
    root = document.querySelector<HTMLElement>("[data-world-app]") as HTMLElement;
    expect(root.dataset.initialized).toBe("true");

    expect(FakeSocket.all).toHaveLength(0);
    expect(status()).toContain("Offline");
    expect(q("[data-world-participants]").textContent).toContain("ada");
    expect(q("[data-world-view]").textContent).toContain("arena view");
    expect(q("[data-world-records]").textContent).toContain("hello");

    // Offline is a reader, not a writer: a chat submit is refused.
    q<HTMLInputElement>("[data-world-chat]").value = "anyone there?";
    q<HTMLFormElement>("[data-world-chat-form]").dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );
    expect(status()).toBe("Join the world before chatting");
    expect(FakeSocket.all).toHaveLength(0);

    // The persisted sequence has no durable journal identity. Reconnect
    // must ask for an authoritative snapshot rather than replay from it.
    q<HTMLInputElement>("[data-world-capability]").value = "guest-capability";
    clickWorldJoin(root);
    expect(FakeSocket.all).toHaveLength(1);
    const socket = FakeSocket.all[0];
    socket.open();
    expect(socket.sent("join")).toHaveLength(1);
    expect(socket.sent("join")[0]).toMatchObject({ after: null });
    expect(socket.sent("join")[0]?.resume_token).toBeUndefined();

    // The journal was rebuilt with overlapping sequence numbers. The
    // host snapshot must replace the offline projection before live events
    // continue from the new history.
    socket.deliver({
      type: "snapshot",
      snapshot: { world_id: "lobby", current_sequence: 3, participants: [{ id: 2, display_name: "grace" }] },
    });
    socket.deliver({ type: "view", data: "rebuilt history view" });
    socket.deliver({ type: "records", records: { note: "rebuilt projection" }, next_after: null });
    socket.deliver({
      type: "event",
      event: { sequence: 4, participant_id: 2, kind: { kind: "chat", data: "continuous" } },
    });

    expect(q("[data-world-participants]").textContent).toContain("grace");
    expect(q("[data-world-participants]").textContent).not.toContain("ada");
    expect(q("[data-world-view]").textContent).toContain("rebuilt history view");
    expect(q("[data-world-records]").textContent).toContain("rebuilt projection");
    expect(q("[data-world-records]").textContent).not.toContain("hello");
    expect(loadWorldSnapshot(localStorage, window.location.host, null)?.sequence).toBe(4);
  });

  it("treats a notification as a hint and rebuilds from a fresh catch-up snapshot", async () => {
    // Oracle: the #10930 binding constraint says a sequence alone cannot
    // prove journal continuity. Even a hint at the same head can refer to
    // a rewritten journal, so clicking it must discard the resume unit and
    // accept the host's fresh projection/view rather than trust local data.
    const previous = FakeSocket.all.at(-1) as FakeSocket;
    previous.deliver({
      type: "snapshot",
      snapshot: { world_id: "lobby", current_sequence: 5, participants: [{ id: 1, display_name: "old ada" }] },
    });
    previous.deliver({ type: "view", data: "old host view" });
    previous.deliver({ type: "records", records: { note: "old projection" }, next_after: null });

    serviceWorker.notify({ type: "client.notify", head: 5, content: "must not be trusted or rendered" });
    await vi.waitFor(() => expect(FakeSocket.all.length).toBeGreaterThan(1));
    const resumed = FakeSocket.all.at(-1) as FakeSocket;
    resumed.open();
    expect(resumed.sent("join")[0]).toMatchObject({ after: null });
    expect(resumed.sent("join")[0]?.resume_token).toBeUndefined();

    // Same sequence, different projection and host-rendered view models a
    // rewritten journal. Fresh host state replaces the local read model.
    resumed.deliver({
      type: "snapshot",
      snapshot: { world_id: "lobby", current_sequence: 5, participants: [{ id: 2, display_name: "new grace" }] },
    });
    resumed.deliver({ type: "view", data: "new host view" });
    resumed.deliver({ type: "records", records: { note: "new projection" }, next_after: null });

    expect(q("[data-world-participants]").textContent).toContain("new grace");
    expect(q("[data-world-participants]").textContent).not.toContain("old ada");
    expect(q("[data-world-view]").textContent).toContain("new host view");
    expect(q("[data-world-records]").textContent).toContain("new projection");
    expect(q("[data-world-records]").textContent).not.toContain("old projection");
    expect(document.body.textContent).not.toContain("must not be trusted or rendered");
  });
});
