import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";

/**
 * #1458 regression tests: the world page's records fetch.
 *
 * Records arrive paginated: the client requests a first page, then
 * asks for each next page until the server answers next_after=null.
 * An input event mid-pagination used to restart the fetch from page
 * one without invalidating the in-flight chain: the stale chain's
 * terminal page then closed the fresh fetch and rendered alone, and
 * the fresh pages were dropped by the recordsPending guard. The fix
 * coalesces a re-request while a fetch is pending and refetches once
 * the in-flight fetch completes.
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

describe("world page records fetch (#1458)", () => {
  const realWebSocket = globalThis.WebSocket;
  const realResizeObserver = globalThis.ResizeObserver;
  let root: HTMLElement;
  let socket: FakeSocket;
  const q = <T extends HTMLElement>(selector: string): T => {
    const found = root.querySelector<T>(selector);
    if (!found) throw new Error(`missing ${selector}`);
    return found;
  };
  const renderedRecords = () => JSON.parse(q("[data-world-records]").textContent ?? "{}") as Record<string, unknown>;
  const inputEvent = (sequence: number) =>
    socket.deliver({
      type: "event",
      event: { sequence, participant_id: 1, kind: { kind: "input", data: [27] } },
    });

  beforeAll(async () => {
    vi.stubGlobal("WebSocket", FakeSocket);
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    sessionStorage.clear();
    document.body.innerHTML = worldPageMarkup();
    await import("./world");
    root = document.querySelector<HTMLElement>("[data-world-app]") as HTMLElement;
    expect(root.dataset.initialized).toBe("true");

    q<HTMLInputElement>("[data-world-capability]").value = "guest-capability";
    q<HTMLButtonElement>("[data-world-join]").click();
    expect(FakeSocket.all).toHaveLength(1);
    socket = FakeSocket.all[0];
    socket.open();
    expect(socket.sent("join")).toHaveLength(1);
    socket.deliver({
      type: "snapshot",
      snapshot: { world_id: "lobby", current_sequence: 4, participants: [{ id: 1, display_name: "ada" }] },
    });
    // The snapshot starts the first records fetch.
    expect(socket.sent("records")).toHaveLength(1);
    expect(socket.sent("records")[0]).not.toHaveProperty("after");
  });

  afterAll(() => {
    vi.stubGlobal("WebSocket", realWebSocket);
    vi.stubGlobal("ResizeObserver", realResizeObserver);
    document.body.innerHTML = "";
  });

  it("control: a two-page fetch renders the complete set", () => {
    socket.deliver({ type: "records", records: { a1: 1 }, next_after: "c1" });
    expect(socket.sent("records")).toHaveLength(2);
    expect(socket.sent("records")[1]).toHaveProperty("after", "c1");

    socket.deliver({ type: "records", records: { a2: 2 }, next_after: null });
    expect(renderedRecords()).toEqual({ a1: 1, a2: 2 });
    // A completed fetch with no intervening re-request sends nothing more.
    expect(socket.sent("records")).toHaveLength(2);
  });

  it("interleave: a re-request mid-pagination cannot render a lone stale page", () => {
    // Start a fresh fetch, as the snapshot/refresh paths do.
    inputEvent(5);
    expect(socket.sent("records")).toHaveLength(3);

    socket.deliver({ type: "records", records: { b1: 10 }, next_after: "c1" });
    expect(socket.sent("records")).toHaveLength(4);
    expect(socket.sent("records")[3]).toHaveProperty("after", "c1");

    // The bug trigger: another input event while page two is in flight.
    // It must NOT send a competing first-page request.
    inputEvent(6);
    expect(socket.sent("records")).toHaveLength(4);

    // The in-flight fetch completes with its complete set...
    socket.deliver({ type: "records", records: { b2: 20 }, next_after: null });
    expect(renderedRecords()).toEqual({ b1: 10, b2: 20 });

    // ...and the coalesced re-request now runs as a fresh first-page fetch.
    expect(socket.sent("records")).toHaveLength(5);
    expect(socket.sent("records")[4]).not.toHaveProperty("after");

    // The fresh fetch reflects the world after the intervening event
    // (b3 appeared) and renders its complete set as the final state.
    socket.deliver({ type: "records", records: { b1: 10, b3: 30 }, next_after: "c2" });
    expect(socket.sent("records")).toHaveLength(6);
    expect(socket.sent("records")[5]).toHaveProperty("after", "c2");
    socket.deliver({ type: "records", records: { b2: 20 }, next_after: null });
    expect(renderedRecords()).toEqual({ b1: 10, b3: 30, b2: 20 });
    expect(socket.sent("records")).toHaveLength(6);
  });

  it("a single-page fetch after the interleave behaves as before", () => {
    q<HTMLButtonElement>("[data-world-records-refresh]").click();
    expect(socket.sent("records")).toHaveLength(7);
    socket.deliver({ type: "records", records: { only: 1 }, next_after: null });
    expect(renderedRecords()).toEqual({ only: 1 });
    expect(socket.sent("records")).toHaveLength(7);
  });

  it("define semantics (#2953): prototype-spelled record keys from a peer frame are stored, not set", () => {
    // The frame is built by JSON.parse so "__proto__" is a genuine own
    // key of the records object, exactly as a peer-supplied frame
    // delivers it. With assignment semantics (Object.assign) the object
    // value would move the records object's prototype and the string
    // value would be silently dropped; neither would render.
    q<HTMLButtonElement>("[data-world-records-refresh]").click();
    const objectFrame = JSON.parse(
      '{"type":"records","records":{"__proto__":{"polluted":"no"},"constructor":"c-value","toString":"t-value","ordinary":1},"next_after":null}',
    ) as Record<string, unknown>;
    socket.deliver(objectFrame);
    const rendered = renderedRecords();
    expect(Object.hasOwn(rendered, "__proto__")).toBe(true);
    expect(rendered["__proto__"]).toEqual({ polluted: "no" });
    expect(rendered["constructor"]).toBe("c-value");
    expect(rendered["toString"]).toBe("t-value");
    expect(rendered["ordinary"]).toBe(1);
    // JSON round-trip preserved every key, and nothing polluted the
    // shared prototype.
    expect(JSON.parse(JSON.stringify(rendered))).toEqual(rendered);
    expect(Object.hasOwn(Object.prototype, "polluted")).toBe(false);
    expect(({} as Record<string, unknown>)["polluted"]).toBeUndefined();

    q<HTMLButtonElement>("[data-world-records-refresh]").click();
    const stringFrame = JSON.parse(
      '{"type":"records","records":{"__proto__":"string-value"},"next_after":null}',
    ) as Record<string, unknown>;
    socket.deliver(stringFrame);
    const renderedString = renderedRecords();
    expect(Object.hasOwn(renderedString, "__proto__")).toBe(true);
    expect(renderedString["__proto__"]).toBe("string-value");
    expect(Object.getPrototypeOf(renderedString)).toBe(Object.prototype);
  });

  it("rejects malformed snapshots without partial state changes and accepts the next valid snapshot", () => {
    const status = q<HTMLOutputElement>("[data-world-status]");
    const participantList = q<HTMLElement>("[data-world-participants]");
    const log = q<HTMLOListElement>("[data-world-log]");
    const recordsRequests = socket.sent("records").length;
    const malformedSnapshots: Array<Record<string, unknown>> = [
      { type: "snapshot" },
      { type: "snapshot", snapshot: null },
      { type: "snapshot", snapshot: { world_id: "bad", participants: [] } },
      { type: "snapshot", snapshot: { world_id: "bad", current_sequence: 90, participants: "bad" } },
      { type: "snapshot", snapshot: { world_id: "bad", current_sequence: Number.NaN, participants: [] } },
      { type: "snapshot", snapshot: { world_id: 3, current_sequence: 90, participants: [] } },
      {
        type: "snapshot",
        snapshot: { world_id: "bad", current_sequence: 90, participants: [{ id: 1, display_name: 2 }] },
      },
    ];

    for (const frame of malformedSnapshots) {
      socket.deliver(frame);
      expect(status.textContent).toBe("Received an invalid world frame");
      expect(status.dataset.state).toBe("error");
      expect(participantList.textContent).toBe("ada");
      expect(socket.sent("records")).toHaveLength(recordsRequests);
    }
    expect(log.textContent).toContain("error: invalid world frame");

    socket.deliver({
      type: "snapshot",
      snapshot: { world_id: "recovered", current_sequence: 8, participants: [{ id: 2, display_name: "grace" }] },
    });
    expect(status.textContent).toBe("Connected");
    expect(status.dataset.state).toBe("good");
    expect(participantList.textContent).toBe("grace");
    expect(log.textContent).toContain("Connected to recovered at event 8");
    expect(socket.sent("records")).toHaveLength(recordsRequests + 1);
  });

  it("rejects wrong-shape records frames (#7908) without rendering, echoing a cursor, or closing the fetch", () => {
    const status = q<HTMLOutputElement>("[data-world-status]");
    const recordsView = q<HTMLElement>("[data-world-records]");
    const renderedBefore = recordsView.textContent;

    q<HTMLButtonElement>("[data-world-records-refresh]").click();
    const requestsBefore = socket.sent("records").length;

    // records that is not a plain object: string renders per-character
    // entries pre-fix, an array per-index entries, a scalar an empty
    // world. A non-string next_after was echoed back as `after`.
    const malformedFrames: Array<Record<string, unknown>> = [
      { type: "records", records: "abc", next_after: null },
      { type: "records", records: [10, 20], next_after: null },
      { type: "records", records: 42, next_after: null },
      { type: "records", records: true, next_after: null },
      { type: "records", records: { x: 1 }, next_after: 123 },
      { type: "records", records: { x: 1 }, next_after: { k: 1 } },
    ];
    for (const frame of malformedFrames) {
      socket.deliver(frame);
      expect(status.textContent).toBe("Received an invalid world frame");
      expect(status.dataset.state).toBe("error");
      // No follow-up page request (a non-string cursor is never
      // echoed) and the rendered records are untouched.
      expect(socket.sent("records")).toHaveLength(requestsBefore);
      expect(recordsView.textContent).toBe(renderedBefore);
    }

    // The fetch is still in flight: the next well-formed frame is
    // accepted and completes it exactly as before. The refresh click
    // above landed while the snapshot test's fetch was in flight, so
    // it was coalesced (#1458): completing this fetch runs that one
    // refetch, as a fresh first-page request with no `after` - never
    // an echo of a rejected cursor.
    socket.deliver({ type: "records", records: { z9: 9 }, next_after: null });
    expect(renderedRecords()).toEqual({ z9: 9 });
    expect(socket.sent("records")).toHaveLength(requestsBefore + 1);
    expect(socket.sent("records").at(-1)).not.toHaveProperty("after");
    socket.deliver({ type: "records", records: { z9: 9 }, next_after: null });
    expect(renderedRecords()).toEqual({ z9: 9 });
    expect(socket.sent("records")).toHaveLength(requestsBefore + 1);
  });

  it("rejects malformed events before cursor or log changes and accepts the next valid event", async () => {
    const status = q<HTMLOutputElement>("[data-world-status]");
    const log = q<HTMLOListElement>("[data-world-log]");
    const recordsRequests = socket.sent("records").length;

    socket.deliver({
      type: "event",
      event: { sequence: 9, participant_id: 2, kind: { kind: "chat", data: "recovered" } },
    });
    expect(log.textContent).toContain("grace: recovered");

    const malformedEvents: Array<Record<string, unknown>> = [
      { type: "event", event: { sequence: 100, participant_id: 2 } },
      { type: "event", event: null },
      { type: "event", event: { sequence: 101, participant_id: 2, kind: null } },
      { type: "event", event: { sequence: 102, participant_id: 2, kind: { kind: "other", data: "bad" } } },
      { type: "event", event: { sequence: 103, participant_id: 2, kind: { kind: "chat", data: 1 } } },
      { type: "event", event: { sequence: 104, participant_id: 2, kind: { kind: "input", data: "bad" } } },
      {
        type: "event",
        event: { sequence: 105, participant_id: 2, kind: { kind: "input", data: [Number.NaN] } },
      },
      { type: "event", event: { sequence: 106, participant_id: "2", kind: { kind: "input", data: [27] } } },
      { type: "event", event: { participant_id: 2, kind: { kind: "chat", data: "bad sequence" } } },
      {
        type: "event",
        event: { sequence: Number.NaN, participant_id: 2, kind: { kind: "chat", data: "bad sequence" } },
      },
    ];

    for (const frame of malformedEvents) {
      socket.deliver(frame);
      expect(status.textContent).toBe("Received an invalid world frame");
      expect(status.dataset.state).toBe("error");
    }
    expect(socket.sent("records")).toHaveLength(recordsRequests);
    expect(log.textContent).not.toContain("input #100");
    expect(log.textContent).not.toContain("bad sequence");

    const closedSocket = socket;
    closedSocket.close();
    await new Promise((resolve) => setTimeout(resolve, 600));
    const reconnectedSocket = FakeSocket.all.at(-1);
    expect(reconnectedSocket).toBeDefined();
    reconnectedSocket!.open();
    expect(reconnectedSocket!.sent("join")[0]).toHaveProperty("after", 9);
  });
});
