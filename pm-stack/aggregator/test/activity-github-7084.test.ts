// #7084 (#3321 GitHub slice): consume the #5204 projector hot list into
// the #1739 activity stream. The oracle is the persisted hot-list consumer
// cursor: it may advance only after every event in one fixed-high-water
// sweep has landed in the activity stream, and replay/restart must stay
// idempotent through the stable `activity:github:<event_id>` ids.
import { describe, expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { type GapRecord, type GithubHotEvent, githubHotEventDelivery, IngestionStore } from "../src/activity.ts";
import { EventsStore } from "../src/events.ts";
import { GithubHotListConsumer } from "../src/github-hot-list.ts";

interface FakeLedger {
  rows: GithubHotEvent[];
  cursor: number;
  consumer: string;
  ackCalls: Array<{ cursor: number; highWater: number }>;
}

function stores(): {
  dir: string;
  eventsFile: string;
  activityFile: string;
  events: EventsStore;
  store: IngestionStore;
} {
  const dir = mkdtempSync(join(tmpdir(), "agg-github-7084-"));
  const eventsFile = join(dir, "events.json");
  const activityFile = join(dir, "activity.json");
  return {
    dir,
    eventsFile,
    activityFile,
    events: new EventsStore(eventsFile),
    store: new IngestionStore(activityFile),
  };
}

function hotEvent(seq: number, id: string, repo = "developing-today/code"): GithubHotEvent {
  return {
    seq,
    event_id: id,
    delivery_id: `delivery-${id}`,
    repo,
    event: "issues",
    action: "opened",
    ref: `${repo}#${seq}`,
    actor: "dezren39",
    source_url: `https://github.com/${repo}/issues/${seq}`,
    source_timestamp: `2026-11-08T12:00:${String(seq % 60).padStart(2, "0")}.000Z`,
    received_at: "2026-11-08T12:01:00.000Z",
  };
}

/** Minimal projector hot-list protocol; a `failAck` injection models the
 *  dangerous window where activity rows are durable but the remote
 *  checkpoint was never acknowledged. */
function projector(rows: GithubHotEvent[], opts: { failAck?: boolean } = {}) {
  const ledger: FakeLedger = { rows: [...rows], cursor: 0, consumer: "unset", ackCalls: [] };
  const fetchMock = (async (input: string | URL | Request, init?: RequestInit) => {
    const url = new URL(String(input));
    const highWater = Math.max(0, ...ledger.rows.map((row) => row.seq));
    if (url.pathname === "/hot/checkpoints" && (!init?.method || init.method === "GET")) {
      return Response.json({ consumer: url.searchParams.get("consumer"), cursor: ledger.cursor });
    }
    if (url.pathname === "/hot" && (!init?.method || init.method === "GET")) {
      ledger.consumer = String(url.searchParams.get("consumer") ?? ledger.consumer);
      const after = Number(url.searchParams.get("after") ?? 0);
      const limit = Number(url.searchParams.get("limit") ?? 200);
      const pinned = Number(url.searchParams.get("high_water") ?? highWater);
      const events = ledger.rows
        .filter((row) => row.seq > after && row.seq <= Math.min(pinned, highWater))
        .sort((a, b) => a.seq - b.seq)
        .slice(0, limit);
      return Response.json({
        events,
        high_water: Math.min(pinned, highWater),
        next_cursor: events.at(-1)?.seq ?? after,
      });
    }
    if (url.pathname === "/hot/checkpoints" && init?.method === "POST") {
      if (opts.failAck) return Response.json({ error: "unavailable" }, { status: 503 });
      const body = JSON.parse(String(init.body)) as { consumer: string; cursor: number; highWater: number };
      if (body.cursor > body.highWater || body.highWater > highWater) {
        return Response.json({ error: "checkpoint_past_high_water" }, { status: 400 });
      }
      ledger.ackCalls.push({ cursor: body.cursor, highWater: body.highWater });
      ledger.cursor = Math.max(ledger.cursor, body.cursor);
      ledger.consumer = body.consumer;
      return Response.json({ consumer: body.consumer, cursor: ledger.cursor });
    }
    return Response.json({ error: "not_found" }, { status: 404 });
  }) as typeof fetch;
  return { ledger, fetchMock };
}

function consumer(
  parts: ReturnType<typeof stores>,
  fetchMock: typeof fetch,
  opts: { pageSize?: number; maxPages?: number } = {},
): GithubHotListConsumer {
  return new GithubHotListConsumer({
    baseUrl: "http://projector.test",
    token: "test-token",
    store: parts.store,
    events: parts.events,
    fetch: fetchMock,
    pageSize: opts.pageSize ?? 2,
    maxPages: opts.maxPages ?? 20,
    now: () => new Date("2026-11-08T12:05:00.000Z"),
  });
}

describe("GitHub hot-list writer (#7084)", () => {
  test("maps a hot-list row to the common event contract with a stable source id and link", () => {
    const row = hotEvent(7, "delivery:issues.opened:1");
    const delivery = githubHotEventDelivery(row);
    expect(delivery).toMatchObject({
      source: "github",
      sourceId: "delivery:issues.opened:1",
      kind: "github-event",
      ref: "developing-today/code#7",
      link: "https://github.com/developing-today/code/issues/7",
    });
    expect(delivery.summary).toContain("issues opened");
    expect(delivery.summary).toContain("dezren39");
    // Same source row -> same delivery; the store persists the
    // `activity:github:<sourceId>` stream id.
    expect(githubHotEventDelivery(row).sourceId).toBe(delivery.sourceId);
  });

  test("rejects a source link outside GitHub and falls back to the repository URL", () => {
    const row = { ...hotEvent(8, "delivery:issues.opened:2"), source_url: "javascript:alert(1)" };
    const delivery = githubHotEventDelivery(row);
    expect(delivery.link).toBe("https://github.com/developing-today/code");
  });
});

describe("GitHub hot-list consumer (#7084)", () => {
  test("loads the existing v1 activity file before source-status metadata existed", async () => {
    const parts = stores();
    writeFileSync(parts.activityFile, JSON.stringify({ version: 1, checkpoints: {}, seen: {}, gaps: [] }), "utf8");
    const legacy = new IngestionStore(parts.activityFile);
    expect(await legacy.sourceStatuses()).toEqual([]);
    await legacy.setSourceStatus({
      source: "github",
      state: "unconfigured",
      consumer: null,
      cursor: null,
      checkpointCursor: null,
      highWater: null,
      lastAttemptAt: null,
      lastSuccessAt: null,
      updatedAt: "2026-11-08T12:00:00.000Z",
    });
    expect((await new IngestionStore(parts.activityFile).sourceStatuses())[0]?.source).toBe("github");
  });

  test("sweeps every page, ingests each row once, then acks the completed high-water", async () => {
    const parts = stores();
    const { ledger, fetchMock } = projector([
      hotEvent(1, "delivery:1"),
      hotEvent(2, "delivery:2"),
      hotEvent(3, "delivery:3"),
    ]);
    const status = await consumer(parts, fetchMock, { pageSize: 2 }).sync();
    expect(status.state).toBe("current");
    expect(status.cursor).toBe(3);
    expect(status.highWater).toBe(3);
    expect(ledger.ackCalls).toEqual([{ cursor: 3, highWater: 3 }]);
    const stream = await parts.events.list();
    expect(stream.map((event) => event.id).sort()).toEqual([
      "activity:github:delivery:1",
      "activity:github:delivery:2",
      "activity:github:delivery:3",
    ]);
    expect(stream.every((event) => event.kind === "github-event")).toBe(true);
    expect(stream.find((event) => event.id === "activity:github:delivery:1")?.summary).toContain(
      "https://github.com/developing-today/code/issues/1",
    );
  });

  test("a duplicate delivery replay is a no-op and the checkpoint still advances", async () => {
    const parts = stores();
    const first = projector([hotEvent(1, "delivery:1"), hotEvent(2, "delivery:2")]);
    await consumer(parts, first.fetchMock).sync();
    expect(await parts.events.list()).toHaveLength(2);
    // The projector loses/replays its checkpoint but reissues the same
    // stable event ids: a fresh local process/store dedupes them too.
    first.ledger.cursor = 0;
    const restarted = {
      ...parts,
      store: new IngestionStore(parts.activityFile),
      events: new EventsStore(parts.eventsFile),
    };
    const replay = await consumer(restarted, first.fetchMock).sync();
    expect(replay.state).toBe("current");
    expect(await restarted.events.list()).toHaveLength(2);
    const checkpoint = await restarted.store.checkpoint("github");
    expect(checkpoint?.duplicates).toBe(2);
    expect(checkpoint?.deliveries).toBe(2);
  });

  test("a failed ack never loses rows: the next sweep replays and advances only then", async () => {
    const parts = stores();
    const broken = projector([hotEvent(1, "delivery:1"), hotEvent(2, "delivery:2")], { failAck: true });
    const failed = await consumer(parts, broken.fetchMock).sync();
    expect(failed.state).toBe("outage");
    expect(broken.ledger.cursor).toBe(0);
    expect(await parts.events.list()).toHaveLength(2);
    const gap = (await parts.store.gaps()).find((row) => row.source === "github");
    expect(gap?.status).toBe("open");

    // Same projector, healthy ack path: the replay dedupes the two rows
    // after a process restart, and only then moves the remote cursor.
    const restarted = {
      ...parts,
      store: new IngestionStore(parts.activityFile),
      events: new EventsStore(parts.eventsFile),
    };
    expect(await restarted.store.sourceStatuses()).toMatchObject([
      { source: "github", state: "outage", cursor: 2, checkpointCursor: 0, highWater: 2 },
    ]);
    const healthy = projector(broken.ledger.rows);
    const recovered = await consumer(restarted, healthy.fetchMock).sync();
    expect(recovered.state).toBe("current");
    expect(healthy.ledger.cursor).toBe(2);
    expect(await restarted.events.list()).toHaveLength(2);
    expect((await restarted.store.gaps()).find((row) => row.source === "github")?.status).toBe("backfilled");
    expect((await restarted.store.checkpoint("github"))?.deliveries).toBe(2);
    expect((await restarted.store.checkpoint("github"))?.duplicates).toBe(0);
  });

  test("restart resumes from the persisted projector checkpoint without re-acking old rows", async () => {
    const parts = stores();
    const { ledger, fetchMock } = projector([hotEvent(1, "delivery:1"), hotEvent(2, "delivery:2")]);
    await consumer(parts, fetchMock).sync();
    const beforeRestart = await parts.events.list();
    ledger.rows.push(hotEvent(3, "delivery:3"));

    // Fresh store + consumer instances over the same files: the projector
    // still holds cursor 2, so only the new row is delivered.
    const store2 = new IngestionStore(parts.activityFile);
    const events2 = new EventsStore(parts.eventsFile);
    const restarted = new GithubHotListConsumer({
      baseUrl: "http://projector.test",
      token: "test-token",
      store: store2,
      events: events2,
      fetch: fetchMock,
      pageSize: 1,
      now: () => new Date("2026-11-08T12:06:00.000Z"),
    });
    const status = await restarted.sync();
    expect(status.state).toBe("current");
    expect(status.cursor).toBe(3);
    expect((await events2.list()).map((event) => event.id).sort()).toEqual(
      [...beforeRestart.map((event) => event.id).sort(), "activity:github:delivery:3"].sort(),
    );
    expect(ledger.ackCalls.map((ack) => ack.cursor)).toEqual([2, 3]);
  });

  test("an exhausted page budget reports lag and never acks a partial sweep", async () => {
    const parts = stores();
    const { ledger, fetchMock } = projector(
      Array.from({ length: 5 }, (_, index) => hotEvent(index + 1, `delivery:${index + 1}`)),
    );
    const status = await consumer(parts, fetchMock, { pageSize: 1, maxPages: 2 }).sync();
    expect(status.state).toBe("lagging");
    expect(status.cursor).toBe(2);
    expect(status.checkpointCursor).toBe(0);
    expect(status.error).toContain("sweep_exceeded_2_pages");
    expect(ledger.ackCalls).toEqual([]);
    expect(await parts.events.list()).toHaveLength(2);
    const gap = (await parts.store.gaps()).find((row) => row.source === "github");
    expect(gap?.status).toBe("open");
    expect(Number.isFinite(Date.parse(gap?.from ?? ""))).toBe(true);
    expect(Number.isFinite(Date.parse(gap?.to ?? ""))).toBe(true);

    const resumed = await consumer(parts, fetchMock, { pageSize: 2, maxPages: 20 }).sync();
    expect(resumed.state).toBe("current");
    expect(resumed.cursor).toBe(5);
    expect(resumed.checkpointCursor).toBe(5);
    expect(ledger.ackCalls).toEqual([{ cursor: 5, highWater: 5 }]);
    expect(await parts.events.list()).toHaveLength(5);
    expect((await parts.store.gaps()).find((row) => row.source === "github")?.status).toBe("backfilled");
  });

  test("a projector outage is exposed as an open github gap with a time range", async () => {
    const parts = stores();
    const failed = (async () => {
      throw new Error("connection refused");
    }) as unknown as typeof fetch;
    const status = await consumer(parts, failed).sync();
    expect(status.state).toBe("outage");
    expect(status.error).toContain("connection refused");
    const gaps = await parts.store.gaps();
    const github = gaps.find((row: GapRecord) => row.source === "github");
    expect(github?.status).toBe("open");
    expect(Number.isFinite(Date.parse(github?.from ?? ""))).toBe(true);
    expect(Number.isFinite(Date.parse(github?.to ?? ""))).toBe(true);
    // The render never presents the source as complete history.
    expect(status.state).not.toBe("current");
  });

  test("a cursor past the hot-list high-water is unrecoverable, never silently reset", async () => {
    const parts = stores();
    const { ledger, fetchMock } = projector([hotEvent(1, "delivery:1")]);
    ledger.cursor = 9;
    const status = await consumer(parts, fetchMock).sync();
    expect(status.state).toBe("unrecoverable");
    expect(status.error).toContain("checkpoint_past");
    expect(ledger.ackCalls).toEqual([]);
    const gap = (await parts.store.gaps()).find((row) => row.source === "github");
    expect(gap?.status).toBe("unrecoverable");
  });
});
