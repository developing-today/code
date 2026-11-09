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
  checkpointViolations: number[];
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
function projector(
  rows: GithubHotEvent[],
  opts: {
    failAck?: boolean;
    beforeAck?: (ack: { cursor: number; highWater: number }) => Promise<boolean>;
  } = {},
) {
  const ledger: FakeLedger = {
    rows: [...rows],
    cursor: 0,
    consumer: "unset",
    ackCalls: [],
    checkpointViolations: [],
  };
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
      if (opts.beforeAck && !(await opts.beforeAck(body))) {
        ledger.checkpointViolations.push(body.cursor);
        return Response.json({ error: "checkpoint_before_rows_durable" }, { status: 409 });
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
  opts: {
    pageSize?: number;
    maxPages?: number;
    events?: Pick<EventsStore, "record">;
    now?: () => Date;
  } = {},
): GithubHotListConsumer {
  return new GithubHotListConsumer({
    baseUrl: "http://projector.test",
    token: "test-token",
    store: parts.store,
    events: opts.events ?? parts.events,
    fetch: fetchMock,
    pageSize: opts.pageSize ?? 2,
    maxPages: opts.maxPages ?? 20,
    now: opts.now ?? (() => new Date("2026-11-08T12:05:00.000Z")),
  });
}

function seededInt(seed: number): (limit: number) => number {
  let state = seed >>> 0;
  return (limit) => {
    state = (Math.imul(state, 1_664_525) + 1_013_904_223) >>> 0;
    return state % limit;
  };
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

  test("maps an oversized push ref to a valid ref without losing it from the summary", () => {
    const branch = `refs/heads/${"feature".repeat(60)}`;
    const delivery = githubHotEventDelivery({
      ...hotEvent(9, "delivery:push:9"),
      event: "push",
      action: null,
      ref: branch,
    });
    expect(delivery.ref).toBe("developing-today/code");
    expect(delivery.summary).toContain("ref refs/heads/");
    expect(delivery.link).toBe("https://github.com/developing-today/code/issues/9");
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

  test("an oversized push ref falls back to the repo ref instead of stalling the sweep", async () => {
    const parts = stores();
    const branch = `refs/heads/${"feature".repeat(60)}`;
    const row = { ...hotEvent(1, "delivery:push:1"), event: "push", action: null, ref: branch };
    const { ledger, fetchMock } = projector([row]);

    const status = await consumer(parts, fetchMock).sync();

    expect(status.state).toBe("current");
    expect(ledger.cursor).toBe(1);
    expect((await parts.events.list())[0]).toMatchObject({
      id: "activity:github:delivery:push:1",
      ref: "developing-today/code",
    });
    expect((await parts.events.list())[0]?.summary).toContain("ref refs/heads/");
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

  test("a repeated failure after an unrecoverable gap creates a distinct, recoverable gap", async () => {
    const parts = stores();
    const baseline = await consumer(parts, projector([]).fetchMock).sync();
    expect(baseline.state).toBe("current");

    const badCheckpoint = projector([hotEvent(1, "delivery:1")]);
    badCheckpoint.ledger.cursor = 9;
    const unrecoverable = await consumer(parts, badCheckpoint.fetchMock).sync();
    expect(unrecoverable.state).toBe("unrecoverable");
    const firstGap = (await parts.store.gaps()).find((gap) => gap.source === "github");
    expect(firstGap?.status).toBe("unrecoverable");

    const failedFetch = (async () => {
      throw new Error("connection refused");
    }) as unknown as typeof fetch;
    expect((await consumer(parts, failedFetch).sync()).state).toBe("outage");
    const secondGap = (await parts.store.gaps()).find((gap) => gap.status === "open");
    expect(secondGap).toBeDefined();
    expect(secondGap?.id).not.toBe(firstGap?.id);

    const recovered = await consumer(parts, projector([hotEvent(1, "delivery:1")]).fetchMock).sync();
    expect(recovered.state).toBe("current");
    expect(await parts.store.gaps()).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ id: firstGap?.id, status: "unrecoverable" }),
        expect.objectContaining({ id: secondGap?.id, status: "backfilled" }),
      ]),
    );
  });

  test("gap resolution prefers an open row in legacy state with a reused id", async () => {
    const parts = stores();
    const id = "gap:github:2026-11-08T12:00:00.000Z";
    writeFileSync(
      parts.activityFile,
      JSON.stringify({
        version: 1,
        checkpoints: {},
        seen: {},
        gaps: [
          {
            id,
            source: "github",
            from: "2026-11-08T12:00:00.000Z",
            to: "2026-11-08T12:01:00.000Z",
            status: "unrecoverable",
            detectedAt: "2026-11-08T12:01:00.000Z",
            resolvedAt: "2026-11-08T12:02:00.000Z",
          },
          {
            id,
            source: "github",
            from: "2026-11-08T12:00:00.000Z",
            to: "2026-11-08T12:03:00.000Z",
            status: "open",
            detectedAt: "2026-11-08T12:03:00.000Z",
          },
        ],
      }),
      "utf8",
    );

    const store = new IngestionStore(parts.activityFile);
    await store.resolveGap(id, "backfilled", "2026-11-08T12:04:00.000Z");

    expect((await store.gaps()).map((gap) => gap.status)).toEqual(["unrecoverable", "backfilled"]);
  });

  test("generated failure/restart schedules never checkpoint undurable rows or duplicate activity", async () => {
    // Deterministic generated traces exercise sparse sequence numbers,
    // varying page sizes, repeated append failures, and process restarts.
    // The oracle checks each remote ack against a fresh read of the durable
    // event log, then checks exact stable-id set equality after recovery.
    for (let seed = 1; seed <= 48; seed++) {
      const draw = seededInt(seed);
      const parts = stores();
      const count = 1 + draw(8);
      let seq = 0;
      const rows = Array.from({ length: count }, (_, index) => {
        seq += 1 + draw(4);
        const kinds = ["issues", "pull_request", "issue_comment", "pull_request_review", "push"];
        const row = hotEvent(seq, `generated:${seed}:${index}`);
        return {
          ...row,
          event: kinds[draw(kinds.length)]!,
          action: draw(4) === 0 ? null : ["opened", "edited", "submitted"][draw(3)]!,
          ref: draw(5) === 0 ? null : row.ref,
        };
      });
      const failedEvent = rows[draw(rows.length)]!;
      const failureAttempts = 1 + draw(3);
      let failuresRemaining = failureAttempts;
      const { ledger, fetchMock } = projector(rows, {
        beforeAck: async ({ cursor }) => {
          const durable = await new EventsStore(parts.eventsFile).list();
          const ids = new Set(durable.map((event) => event.id));
          return rows
            .filter((row) => row.seq <= cursor)
            .every((row) => ids.has(`activity:github:${row.event_id}`));
        },
      });

      for (let attempt = 0; attempt < failureAttempts; attempt++) {
        const restarted = {
          ...parts,
          store: new IngestionStore(parts.activityFile),
          events: new EventsStore(parts.eventsFile),
        };
        const failOnce: Pick<EventsStore, "record"> = {
          record: async (event) => {
            if (event.id === `activity:github:${failedEvent.event_id}` && failuresRemaining > 0) {
              failuresRemaining--;
              throw new Error(`generated_append_failure:${seed}:${attempt}`);
            }
            return restarted.events.record(event);
          },
        };
        const status = await consumer(restarted, fetchMock, {
          events: failOnce,
          pageSize: 1 + draw(4),
          maxPages: 40,
        }).sync();
        expect(status.state).toBe("outage");
        expect(ledger.cursor).toBe(0);
        expect(ledger.ackCalls).toEqual([]);
        expect(ledger.checkpointViolations).toEqual([]);
      }

      const recovered = {
        ...parts,
        store: new IngestionStore(parts.activityFile),
        events: new EventsStore(parts.eventsFile),
      };
      const status = await consumer(recovered, fetchMock, {
        pageSize: 1 + draw(4),
        maxPages: 40,
      }).sync();
      expect(status.state).toBe("current");
      expect(ledger.cursor).toBe(seq);
      expect(ledger.ackCalls).toEqual([{ cursor: seq, highWater: seq }]);
      expect(ledger.checkpointViolations).toEqual([]);
      expect((await recovered.events.list()).map((event) => event.id).sort()).toEqual(
        rows.map((row) => `activity:github:${row.event_id}`).sort(),
      );
    }
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
