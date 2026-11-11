import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";

/**
 * #7924 regression tests: the world page's session-WS invite, view and
 * error frames were shape-trusted. A non-string invite capability was
 * coerced and persisted to sessionStorage as '[object Object]' with an
 * 'Invite ready to share' status, non-string view data rendered coerced
 * and a non-number view_revision was stored verbatim, and a non-string
 * error message displayed coerced. Malformed frames must now be
 * rejected (the snapshot/event rejectFrame() discipline) without
 * changing persisted or rendered state; well-formed frames behave
 * exactly as before.
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

describe("world page invite/view/error frame shapes (#7924)", () => {
  const realWebSocket = globalThis.WebSocket;
  const realResizeObserver = globalThis.ResizeObserver;
  let root: HTMLElement;
  let socket: FakeSocket;
  const q = <T extends HTMLElement>(selector: string): T => {
    const found = root.querySelector<T>(selector);
    if (!found) throw new Error(`missing ${selector}`);
    return found;
  };
  const storageKey = () => `id-world-capability:${window.location.host}`;
  const status = () => q<HTMLElement>("[data-world-status]");
  const capability = () => q<HTMLInputElement>("[data-world-capability]");
  const inviteOutput = () => q<HTMLElement>("[data-world-invite-output]");
  const view = () => q<HTMLElement>("[data-world-view]");
  const log = () => q<HTMLOListElement>("[data-world-log]");

  beforeAll(async () => {
    vi.stubGlobal("WebSocket", FakeSocket);
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    sessionStorage.clear();
    document.body.innerHTML = worldPageMarkup();
    await import("./world");
    root = document.querySelector<HTMLElement>("[data-world-app]") as HTMLElement;
    expect(root.dataset.initialized).toBe("true");
    capability().value = "guest-capability";
    q<HTMLButtonElement>("[data-world-join]").click();
    expect(FakeSocket.all).toHaveLength(1);
    socket = FakeSocket.all[0];
    socket.open();
    socket.deliver({
      type: "snapshot",
      snapshot: { world_id: "lobby", current_sequence: 4, participants: [{ id: 1, display_name: "ada" }] },
    });
    expect(status().textContent).toBe("Connected");
  });

  afterAll(() => {
    vi.stubGlobal("WebSocket", realWebSocket);
    vi.stubGlobal("ResizeObserver", realResizeObserver);
    document.body.innerHTML = "";
  });

  it("control: a well-formed invite stores the capability and reports ready", () => {
    socket.deliver({ type: "invite", invite: { capability: "cap-valid-123", display_name: "ada" } });
    expect(capability().value).toBe("cap-valid-123");
    expect(sessionStorage.getItem(storageKey())).toBe("cap-valid-123");
    expect(inviteOutput().hidden).toBe(false);
    expect(status().textContent).toBe("Invite ready to share");
    expect(status().dataset.state).toBe("good");
  });

  it("rejects object and numeric invite capabilities without changing persisted state", () => {
    const malformed: Array<Record<string, unknown>> = [
      { type: "invite", invite: { capability: { forged: true } } },
      { type: "invite", invite: { capability: 12345 } },
      { type: "invite", capability: { forged: true } },
      { type: "invite", capability: 12345 },
    ];
    for (const frame of malformed) {
      inviteOutput().hidden = true;
      socket.deliver(frame);
      expect(status().textContent).toBe("Received an invalid world frame");
      expect(status().dataset.state).toBe("error");
      expect(capability().value).toBe("cap-valid-123");
      expect(sessionStorage.getItem(storageKey())).toBe("cap-valid-123");
      expect(inviteOutput().hidden).toBe(true);
    }
    expect(log().textContent).not.toContain("[object Object]");
  });

  it("an invite with a missing capability stays a no-op", () => {
    const before = status().textContent;
    socket.deliver({ type: "invite", invite: {} });
    expect(capability().value).toBe("cap-valid-123");
    expect(sessionStorage.getItem(storageKey())).toBe("cap-valid-123");
    expect(status().textContent).toBe(before);
  });

  it("control: a well-formed view data frame renders, then malformed view frames are rejected", () => {
    socket.deliver({ type: "view", data: "hello world", view_revision: 3 });
    expect(view().textContent).toBe("hello world");
    const malformed: Array<Record<string, unknown>> = [
      { type: "view", data: { not: "text" } },
      { type: "view", data: "should not render", view_revision: "7" },
      { type: "view", data: 42 },
    ];
    for (const frame of malformed) {
      socket.deliver(frame);
      expect(status().textContent).toBe("Received an invalid world frame");
      expect(view().textContent).toBe("hello world");
    }
    expect(log().textContent).not.toContain("[object Object]");
    socket.deliver({ type: "view", data: "recovered view", view_revision: 4 });
    expect(view().textContent).toBe("recovered view");
  });

  it("control: a string error message displays, an object error message is rejected uncoerced", () => {
    socket.deliver({ type: "error", message: "boom" });
    expect(status().textContent).toBe("boom");
    expect(status().dataset.state).toBe("error");
    socket.deliver({ type: "error", message: { code: 42 } });
    expect(status().textContent).toBe("Received an invalid world frame");
    expect(status().textContent).not.toContain("[object Object]");
    expect(log().textContent).not.toContain("[object Object]");
    socket.deliver({ type: "error" });
    expect(status().textContent).toBe("World request failed");
  });
  it("property: generated invite/view/error frame shapes follow the acceptance oracle; rejected frames preserve state", () => {
    // Oracle — the #7924 validation in world.ts receive(), restated:
    // - invite: reject iff `invite` is present and not a record, or a
    //   present non-null capability / display_name is not a string.
    // - view: reject iff a present data is not a string, or a present
    //   non-null view_revision / base_view_revision is not a finite number.
    // - error: reject iff a present non-null message is not a string.
    // A rejected frame may set only the invalid-frame status: capability,
    // sessionStorage and the rendered view stay byte-identical. An
    // accepted frame applies exactly the field effects the oracle names.
    // The generator is a fixed-seed PRNG, so the run is deterministic.
    let seed = 0x7924;
    const rand = () => {
      seed = (seed + 0x6d2b79f5) | 0;
      let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
    const values: unknown[] = ["text", "second", "", 42, 3.5, 0, true, false, null, { nested: [1, 2] }, ["a"], {}];
    const pick = () => values[Math.floor(rand() * values.length)];
    const isRec = (v: unknown): v is Record<string, unknown> =>
      typeof v === "object" && v !== null && !Array.isArray(v);
    const badStringField = (v: unknown) => v !== undefined && v !== null && typeof v !== "string";
    const badNumberField = (v: unknown) =>
      v !== undefined && v !== null && !(typeof v === "number" && Number.isFinite(v));
    const seen = { inviteRejected: 0, inviteAccepted: 0, viewRejected: 0, viewAccepted: 0, errorRejected: 0, errorAccepted: 0, displayName: 0, baseRevision: 0 };

    for (let i = 0; i < 240; i++) {
      const capBefore = capability().value;
      const storedBefore = sessionStorage.getItem(storageKey());
      const viewBefore = view().textContent;
      const kind = i % 3;
      if (kind === 0) {
        const shape = rand();
        let frame: Record<string, unknown>;
        let inv: Record<string, unknown>;
        let inviteIsRecord = true;
        if (shape < 0.65) {
          const inner: Record<string, unknown> = {};
          if (rand() < 0.9) inner.capability = pick();
          if (rand() < 0.7) { inner.display_name = pick(); seen.displayName++; }
          frame = { type: "invite", invite: inner };
          inv = inner;
        } else if (shape < 0.85) {
          frame = { type: "invite", capability: pick() };
          if (rand() < 0.7) { frame.display_name = pick(); seen.displayName++; }
          inv = frame;
        } else {
          const bad = pick();
          frame = { type: "invite", invite: bad };
          inv = {};
          inviteIsRecord = isRec(bad);
          if (inviteIsRecord) inv = bad as Record<string, unknown>;
        }
        const rejected = !inviteIsRecord || badStringField(inv.capability) || badStringField(inv.display_name);
        socket.deliver(frame);
        if (rejected) {
          seen.inviteRejected++;
          expect(status().textContent).toBe("Received an invalid world frame");
          expect(capability().value).toBe(capBefore);
          expect(sessionStorage.getItem(storageKey())).toBe(storedBefore);
          expect(view().textContent).toBe(viewBefore);
        } else {
          seen.inviteAccepted++;
          if (typeof inv.capability === "string" && inv.capability) {
            expect(capability().value).toBe(inv.capability);
            expect(sessionStorage.getItem(storageKey())).toBe(inv.capability);
            expect(status().textContent).toBe("Invite ready to share");
          } else {
            expect(capability().value).toBe(capBefore);
            expect(sessionStorage.getItem(storageKey())).toBe(storedBefore);
          }
          expect(view().textContent).toBe(viewBefore);
        }
      } else if (kind === 1) {
        const frame: Record<string, unknown> = { type: "view" };
        if (rand() < 0.9) frame.data = pick();
        if (rand() < 0.7) frame.view_revision = pick();
        if (rand() < 0.5) { frame.base_view_revision = pick(); seen.baseRevision++; }
        const rejected =
          (frame.data !== undefined && typeof frame.data !== "string") ||
          badNumberField(frame.view_revision) ||
          badNumberField(frame.base_view_revision);
        socket.deliver(frame);
        if (rejected) {
          seen.viewRejected++;
          expect(status().textContent).toBe("Received an invalid world frame");
          expect(view().textContent).toBe(viewBefore);
        } else {
          seen.viewAccepted++;
          if (typeof frame.data === "string") expect(view().textContent).toBe(frame.data);
          else expect(view().textContent).toBe(viewBefore);
        }
        expect(capability().value).toBe(capBefore);
        expect(sessionStorage.getItem(storageKey())).toBe(storedBefore);
      } else {
        const frame: Record<string, unknown> = { type: "error" };
        if (rand() < 0.9) frame.message = pick();
        const rejected = badStringField(frame.message);
        socket.deliver(frame);
        if (rejected) {
          seen.errorRejected++;
          expect(status().textContent).toBe("Received an invalid world frame");
        } else {
          seen.errorAccepted++;
          if (typeof frame.message === "string") expect(status().textContent).toBe(frame.message);
          else expect(status().textContent).toBe("World request failed");
        }
        expect(capability().value).toBe(capBefore);
        expect(sessionStorage.getItem(storageKey())).toBe(storedBefore);
        expect(view().textContent).toBe(viewBefore);
      }
    }
    // Coverage guard: the generated run must actually exercise both
    // outcomes per frame kind and both branches Codex named.
    expect(seen.inviteRejected).toBeGreaterThan(5);
    expect(seen.inviteAccepted).toBeGreaterThan(3);
    expect(seen.viewRejected).toBeGreaterThan(5);
    expect(seen.viewAccepted).toBeGreaterThan(3);
    expect(seen.errorRejected).toBeGreaterThan(5);
    expect(seen.errorAccepted).toBeGreaterThan(3);
    expect(seen.displayName).toBeGreaterThan(10);
    expect(seen.baseRevision).toBeGreaterThan(10);
  });
});
