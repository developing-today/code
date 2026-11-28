import { describe, expect, test } from "bun:test";
import {
  FRESHNESS_PAGE_SCOPES,
  createFreshnessStream,
  deliveryScopeFromUrl,
  freshnessDeliveryResponse,
  freshnessScopeAllowed,
  parseAfter,
  readEnvelope,
  sameOrigin,
  type FreshnessJournalReader,
} from "../src/freshness-delivery.ts";
import type { FreshnessNotice } from "../src/freshness.ts";

function notice(sequence: number, scope: string[], source: FreshnessNotice["source"] = "github"): FreshnessNotice {
  return {
    eventId: `event-${sequence}`,
    source,
    scope,
    occurredAt: "2026-11-10T19:00:00.000Z",
    reason: "source-event",
    schemaVersion: 1,
    sequence,
    observedAt: "2026-11-10T19:00:01.000Z",
  };
}

/** Fixed journal: sequences > `after`, honouring the same allow-list filter
 *  contract as FreshnessStore.listAfter. */
function reader(events: FreshnessNotice[], options: { resetRequired?: boolean; cursor?: number } = {}): FreshnessJournalReader {
  const cursor = options.cursor ?? events.at(-1)?.sequence ?? 0;
  return {
    async listAfter(after: number) {
      return {
        cursor,
        resetRequired: options.resetRequired ?? false,
        events: events.filter((event) => event.sequence > after),
      };
    },
  };
}

/** Reads SSE frames until `predicate` matches or the stream is aborted. */
async function readFrames(
  response: Response,
  predicate: (text: string) => boolean,
  signal: AbortSignal,
): Promise<string> {
  const body = response.body;
  if (body === null) throw new Error("no body");
  const readerRef = body.getReader();
  const decoder = new TextDecoder();
  let text = "";
  try {
    for (;;) {
      if (signal.aborted) break;
      const { done, value } = await readerRef.read();
      if (done) break;
      text += decoder.decode(value, { stream: true });
      if (predicate(text)) break;
    }
  } finally {
    readerRef.releaseLock();
    await body.cancel().catch(() => {});
  }
  return text;
}

describe("freshness delivery scope", () => {
  test("derives scope from a known page name only", () => {
    const board = deliveryScopeFromUrl(new URL("http://agg.test/freshness/stream?page=board"));
    expect(board.ok).toBe(true);
    if (board.ok) {
      expect(board.scope.page).toBe("board");
      expect([...board.scope.scopes!].sort()).toEqual([...FRESHNESS_PAGE_SCOPES.board!].sort());
    }
  });

  test("rejects an unknown page instead of falling back to every scope", () => {
    const result = deliveryScopeFromUrl(new URL("http://agg.test/freshness/stream?page=admin"));
    expect(result).toEqual({ ok: false, status: 400, error: "unknown_page" });
  });

  test("accepts only allow-listed journal scopes and rejects the rest", () => {
    expect(freshnessScopeAllowed("board")).toBe(true);
    expect(freshnessScopeAllowed("feed")).toBe(true);
    expect(freshnessScopeAllowed("developing-today/code#3978")).toBe(true);
    expect(freshnessScopeAllowed("*")).toBe(false);
    expect(freshnessScopeAllowed("secret-scope")).toBe(false);
    expect(freshnessScopeAllowed("developing-today/code#0")).toBe(false);

    const ok = deliveryScopeFromUrl(new URL("http://agg.test/api/freshness-events.json?scope=board,feed"));
    expect(ok.ok).toBe(true);
    const bad = deliveryScopeFromUrl(new URL("http://agg.test/api/freshness-events.json?scope=everything"));
    expect(bad).toEqual({ ok: false, status: 400, error: "invalid_scope" });
  });

  test("a bare request sees every allow-listed scope, never an unbounded one", () => {
    const result = deliveryScopeFromUrl(new URL("http://agg.test/api/freshness-events.json"));
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.scope.scopes).toBeNull();
      expect(result.scope.page).toBeNull();
    }
  });

  test("parses cursors strictly", () => {
    expect(parseAfter(null)).toBe(0);
    expect(parseAfter("")).toBe(0);
    expect(parseAfter("0")).toBe(0);
    expect(parseAfter("42")).toBe(42);
    expect(parseAfter("-1")).toBeNull();
    expect(parseAfter("1.5")).toBeNull();
    expect(parseAfter("abc")).toBeNull();
    expect(parseAfter("9007199254740992")).toBeNull();
  });
});

describe("freshness envelope", () => {
  test("filters events by page scope, reports the cursor, and keeps envelope parity", async () => {
    const store = reader([
      notice(1, ["board"]),
      notice(2, ["feed"], "fleet-feed"),
      notice(3, ["developing-today/code#3978"]),
    ]);
    const scope = deliveryScopeFromUrl(new URL("http://agg.test/x?page=board"));
    if (!scope.ok) throw new Error("scope");
    const envelope = await readEnvelope(store, scope.scope, 0);

    expect(envelope.schemaVersion).toBe(1);
    expect(envelope.cursor).toBe(3);
    expect(envelope.resetRequired).toBe(false);
    expect(envelope.events.map((event) => event.sequence)).toEqual([1]);
    expect(envelope.sources.github.notices).toBe(2);
    expect(envelope.sources["fleet-feed"].notices).toBe(1);
    // Every source is present, even with no notices, so a client never has to
    // distinguish "absent" from "healthy".
    expect(Object.keys(envelope.sources).sort()).toEqual(["fleet-feed", "github", "kaneo", "orchestration", "reconcile"]);
  });

  test("marks a source live only while a retained notice is recent", async () => {
    const store = reader([notice(1, ["board"]), notice(2, ["feed"], "kaneo")]);
    const now = new Date("2026-11-10T19:00:30.000Z"); // 29s after observedAt
    const scope = deliveryScopeFromUrl(new URL("http://agg.test/x"));
    if (!scope.ok) throw new Error("scope");
    const live = await readEnvelope(store, scope.scope, 0, now);
    expect(live.sources.github.state).toBe("live");
    expect(live.sources.kaneo.state).toBe("live");
    expect(live.sources.reconcile.state).toBe("silent");
    expect(live.sources.reconcile.lastObservedAt).toBeNull();

    const later = await readEnvelope(store, scope.scope, 0, new Date("2026-11-10T21:00:00.000Z"));
    expect(later.sources.github.state).toBe("stale");
  });

  test("carries the store's reset signal without inventing events", async () => {
    const store = reader([], { resetRequired: true, cursor: 900 });
    const scope = deliveryScopeFromUrl(new URL("http://agg.test/x?page=board"));
    if (!scope.ok) throw new Error("scope");
    const envelope = await readEnvelope(store, scope.scope, 1);
    expect(envelope.resetRequired).toBe(true);
    expect(envelope.cursor).toBe(900);
    expect(envelope.events).toEqual([]);
  });
});

describe("freshness SSE stream", () => {
  test("delivers retained notices with journal sequence ids, then a coalesced patch", async () => {
    const signal = new AbortController();
    const store = reader([notice(1, ["board"]), notice(2, ["board"]), notice(3, ["feed"], "fleet-feed")]);
    const scope = deliveryScopeFromUrl(new URL("http://agg.test/freshness/stream?page=board"));
    if (!scope.ok) throw new Error("scope");
    const response = createFreshnessStream({ reader: store, scope: scope.scope, after: 0, pollMs: 5, heartbeatMs: 50, signal: signal.signal });

    expect(response.status).toBe(200);
    expect(response.headers.get("content-type")).toContain("text/event-stream");
    expect(response.headers.get("cache-control")).toContain("no-cache");

    const text = await readFrames(response, (current) => current.includes("datastar-patch-elements"), signal.signal);
    signal.abort();

    expect(text).toContain("id: 1\nevent: freshness");
    expect(text).toContain("id: 2\nevent: freshness");
    // The feed-scoped notice is not visible to the board page.
    expect(text).not.toContain("id: 3");
    // One render patch for the whole tick, in the existing Datastar format.
    expect(text.match(/event: datastar-patch-elements/g)).toHaveLength(1);
    expect(text).toContain("data: selector #board-main");
    expect(text).toContain("data: mode outer");
  });

  test("reconnect honors Last-Event-ID by replaying only later retained notices", async () => {
    const signal = new AbortController();
    const store = reader([notice(1, ["board"]), notice(2, ["board"]), notice(3, ["board"])]);
    const scope = deliveryScopeFromUrl(new URL("http://agg.test/freshness/stream?page=board"));
    if (!scope.ok) throw new Error("scope");
    const response = await freshnessDeliveryResponse(
      new Request("http://agg.test/freshness/stream?page=board&after=0", { headers: { "last-event-id": "2" } }),
      new URL("http://agg.test/freshness/stream?page=board&after=0"),
      { reader: store, pollMs: 5, heartbeatMs: 50, signal: signal.signal },
    );
    if (response === null) throw new Error("route");

    const text = await readFrames(response, (current) => current.includes("datastar-patch-elements"), signal.signal);
    signal.abort();
    expect(text).not.toContain("id: 1\n");
    expect(text).not.toContain("id: 2\n");
    expect(text).toContain("id: 3\nevent: freshness");
  });

  test("sends one reset event when the cursor predates the retained journal", async () => {
    const signal = new AbortController();
    const store = reader([], { resetRequired: true, cursor: 500 });
    const scope = deliveryScopeFromUrl(new URL("http://agg.test/freshness/stream?page=board"));
    if (!scope.ok) throw new Error("scope");
    const response = createFreshnessStream({ reader: store, scope: scope.scope, after: 1, pollMs: 5, heartbeatMs: 50, signal: signal.signal });

    const text = await readFrames(response, (current) => current.includes("event: reset"), signal.signal);
    signal.abort();
    expect(text).toContain("event: reset");
    expect(text).toContain('"resetRequired":true');
    expect(text).toContain('"cursor":500');
    // No Datastar patch: a reset makes the client refetch its whole view.
    expect(text).not.toContain("datastar-patch-elements");
  });

  test("heartbeat keeps an idle connection alive without claiming freshness", async () => {
    const signal = new AbortController();
    const store = reader([]);
    const scope = deliveryScopeFromUrl(new URL("http://agg.test/freshness/stream?page=board"));
    if (!scope.ok) throw new Error("scope");
    const response = createFreshnessStream({ reader: store, scope: scope.scope, after: 0, pollMs: 5, heartbeatMs: 10, signal: signal.signal });

    const text = await readFrames(response, (current) => /: heartbeat /.test(current), signal.signal);
    signal.abort();
    expect(text).toMatch(/: heartbeat 20\d\d-/);
    expect(text).not.toContain("event: freshness");
    expect(text).not.toContain("event: reset");
  });

  test("a later write reaches an open stream", async () => {
    const signal = new AbortController();
    const events: FreshnessNotice[] = [];
    let cursor = 0;
    const live: FreshnessJournalReader = {
      async listAfter(after: number) {
        return { cursor, resetRequired: false, events: events.filter((event) => event.sequence > after) };
      },
    };
    const scope = deliveryScopeFromUrl(new URL("http://agg.test/freshness/stream?page=board"));
    if (!scope.ok) throw new Error("scope");
    const response = createFreshnessStream({ reader: live, scope: scope.scope, after: 0, pollMs: 5, heartbeatMs: 1000, signal: signal.signal });

    const bodyPromise = readFrames(response, (current) => current.includes("id: 7"), signal.signal);
    events.push(notice(7, ["board"]));
    cursor = 7;
    const text = await bodyPromise;
    signal.abort();
    expect(text).toContain("id: 7\nevent: freshness");
  });
});

describe("freshness delivery routes", () => {
  test("serves the JSON twin with the same envelope and cursor", async () => {
    const store = reader([notice(1, ["board"]), notice(2, ["developing-today/code#3978"])]);
    const url = new URL("http://agg.test/api/freshness-events.json?after=0&page=board");
    const response = await freshnessDeliveryResponse(new Request(url), url, { reader: store });
    if (response === null) throw new Error("route");
    expect(response.status).toBe(200);
    expect(response.headers.get("content-type")).toContain("application/json");
    const envelope = (await response.json()) as { cursor: number; events: FreshnessNotice[]; schemaVersion: number };
    expect(envelope.schemaVersion).toBe(1);
    expect(envelope.cursor).toBe(2);
    expect(envelope.events.map((event) => event.sequence)).toEqual([1]);
  });

  test("the JSON twin honors after= as its cursor", async () => {
    const store = reader([notice(1, ["board"]), notice(2, ["board"]), notice(3, ["board"])]);
    const url = new URL("http://agg.test/api/freshness-events.json?after=1");
    const response = await freshnessDeliveryResponse(new Request(url), url, { reader: store });
    if (response === null) throw new Error("route");
    const envelope = (await response.json()) as { cursor: number; events: FreshnessNotice[] };
    expect(envelope.cursor).toBe(3);
    expect(envelope.events.map((event) => event.sequence)).toEqual([2, 3]);
  });

  test("the stream honors ?after= when no Last-Event-ID header is sent", async () => {
    const signal = new AbortController();
    const store = reader([notice(1, ["board"]), notice(2, ["board"]), notice(3, ["board"])]);
    const url = new URL("http://agg.test/freshness/stream?page=board&after=2");
    const response = await freshnessDeliveryResponse(new Request(url), url, {
      reader: store,
      pollMs: 5,
      heartbeatMs: 50,
      signal: signal.signal,
    });
    if (response === null) throw new Error("route");
    const text = await readFrames(response, (current) => current.includes("datastar-patch-elements"), signal.signal);
    signal.abort();
    expect(text).not.toContain("id: 1\n");
    expect(text).not.toContain("id: 2\n");
    expect(text).toContain("id: 3\nevent: freshness");
  });

  test("returns null for unrelated paths so the route table chains", async () => {
    const url = new URL("http://agg.test/api/board.json");
    expect(await freshnessDeliveryResponse(new Request(url), url, { reader: reader([]) })).toBeNull();
  });

  test("rejects writes, bad cursors, unknown pages, and cross-origin callers", async () => {
    const store = reader([notice(1, ["board"])]);
    const post = new URL("http://agg.test/freshness/stream?page=board");
    const posted = await freshnessDeliveryResponse(new Request(post, { method: "POST" }), post, { reader: store });
    expect(posted?.status).toBe(405);

    const badAfter = new URL("http://agg.test/api/freshness-events.json?after=soon");
    expect((await freshnessDeliveryResponse(new Request(badAfter), badAfter, { reader: store }))?.status).toBe(400);

    const badPage = new URL("http://agg.test/api/freshness-events.json?page=nope");
    expect((await freshnessDeliveryResponse(new Request(badPage), badPage, { reader: store }))?.status).toBe(400);

    const crossOrigin = new URL("http://agg.test/api/freshness-events.json");
    const response = await freshnessDeliveryResponse(
      new Request(crossOrigin, { headers: { origin: "https://evil.test" } }),
      crossOrigin,
      { reader: store },
    );
    expect(response?.status).toBe(403);
  });

  test("same-origin check passes navigations and the tailnet origin, denies others", () => {
    const url = new URL("http://100.64.0.9:4870/freshness/stream");
    expect(sameOrigin(new Request(url), url)).toBe(true);
    expect(sameOrigin(new Request(url, { headers: { origin: "http://100.64.0.9:4870" } }), url)).toBe(true);
    expect(sameOrigin(new Request(url, { headers: { origin: "http://100.64.0.9:9999" } }), url)).toBe(false);
    expect(sameOrigin(new Request(url, { headers: { origin: "null" } }), url)).toBe(false);
  });

  test("a caller-supplied authorize decision is honored (no implicit bypass)", async () => {
    const store = reader([notice(1, ["board"])]);
    const url = new URL("http://agg.test/api/freshness-events.json");
    const denied = await freshnessDeliveryResponse(new Request(url), url, { reader: store, authorize: () => false });
    expect(denied?.status).toBe(403);
    const allowed = await freshnessDeliveryResponse(new Request(url), url, { reader: store, authorize: () => true });
    expect(allowed?.status).toBe(200);
  });

  test("never accepts a token, actor, or payload in the query", async () => {
    const store = reader([notice(1, ["board"])]);
    const url = new URL("http://agg.test/api/freshness-events.json?token=abc&actor=drew&payload=%7B%7D");
    const response = await freshnessDeliveryResponse(new Request(url), url, { reader: store });
    const body = await response!.text();
    expect(body).not.toContain("abc");
    expect(body).not.toContain("drew");
    expect(body).not.toContain("payload");
  });
});
