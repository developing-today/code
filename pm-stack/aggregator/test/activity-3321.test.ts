// #3321: activity ingestion — watchdog-event and Kaneo-change writers
// into the #1739 stream, with per-source persisted checkpoints,
// duplicate-delivery dedupe, restart resume, and gap records that a
// backfill flips. GitHub stays unsupported pending #2072 (asserted in
// the coverage table so the gap is documented, not implied away).
import { describe, expect, test } from "bun:test";
import { existsSync, mkdtempSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  ACTIVITY_SOURCE_COVERAGE,
  IngestionStore,
  WatchdogEpisodeStabilizer,
  activityCoverageHtml,
  createWatchdogGapSyncQueue,
  gapsHtml,
  ingestActivity,
  kaneoChangeDeliveries,
  maxUndatedSeqByCard,
  syncWatchdogGaps,
  watchdogDeliveries,
  type ActivityDelivery,
  type KaneoCardSnapshot,
} from "../src/activity.ts";
import { EventsStore } from "../src/events.ts";
import type { TierItem } from "../src/model.ts";

function tmpStores(): { eventsFile: string; activityFile: string; events: EventsStore; store: IngestionStore } {
  const dir = mkdtempSync(join(tmpdir(), "agg-activity-"));
  const eventsFile = join(dir, "events.json");
  const activityFile = join(dir, "activity.json");
  return { eventsFile, activityFile, events: new EventsStore(eventsFile), store: new IngestionStore(activityFile) };
}

function validIngestionFile() {
  return {
    version: 1,
    checkpoints: {
      kaneo: {
        source: "kaneo",
        lastSourceId: "kaneo:card-1",
        lastEventAt: "2024-02-29T10:00:00.000Z",
        deliveries: 2,
        duplicates: 1,
        updatedAt: "2024-02-29T10:00:00.000Z",
      },
      watchdog: {
        source: "watchdog",
        lastSourceId: null,
        lastEventAt: null,
        deliveries: 0,
        duplicates: 0,
        updatedAt: "2024-02-29T10:00:00.000Z",
      },
    },
    seen: { kaneo: ["kaneo:card-1"] },
    gaps: [
      {
        id: "gap:kaneo:2024-02-29",
        source: "kaneo",
        from: "2024-02-29T10:00:00.000Z",
        to: "2024-03-01T10:00:00.000Z",
        status: "backfilled" as const,
        detectedAt: "2024-02-29T10:00:00.000Z",
        resolvedAt: "2024-03-01T10:00:00.000Z",
      },
      {
        id: "gap:watchdog:2024-03-02",
        source: "watchdog",
        from: "2024-03-02T10:00:00.000Z",
        to: "2024-03-03T10:00:00.000Z",
        status: "open" as const,
        detectedAt: "2024-03-02T10:00:00.000Z",
      },
    ],
  };
}

const iso = (msAgo: number) => new Date(Date.now() - msAgo).toISOString();

function watchdogItem(id: string, sinceMsAgo: number, title = "Watchdog: fleet feed is stale"): TierItem {
  return {
    id,
    kind: "gap",
    title,
    source: "watchdog",
    state: "watchdog: feed stale",
    reason: "test",
    since: iso(sinceMsAgo),
    lastSeenAt: iso(0),
    ref: "developing-today/code#1148",
    recommendedAction: "test",
  };
}

const kaneoCard = (id: string, status: string, updatedMsAgo = 60_000): KaneoCardSnapshot => ({
  id,
  status,
  title: `Card ${id}`,
  updatedAt: iso(updatedMsAgo),
  kaneoUrl: `https://kaneo.security.cab/dashboard/workspace/ws/project/p/task/${id}`,
});

describe("activity writers (#3321)", () => {
  test("watchdog deliveries carry stable per-episode source ids", () => {
    const item = watchdogItem("watchdog:fleet-feed", 300_000);
    const [d] = watchdogDeliveries([item]);
    expect(d?.source).toBe("watchdog");
    expect(d?.kind).toBe("watchdog-event");
    expect(d?.ref).toBe("watchdog:fleet-feed");
    expect(d?.sourceId).toBe(`watchdog:fleet-feed@${item.since}`);
    // Same episode re-derived -> identical delivery (dedupe can work).
    expect(watchdogDeliveries([item])[0]?.sourceId).toBe(d?.sourceId);
  });

  test("kaneo diff: appearance and status change emit; unchanged is silent", () => {
    const a = kaneoCard("kaneo:p/t1", "in-progress");
    const b = kaneoCard("kaneo:p/t2", "todo");
    const prev = new Map([[a.id, a]]);
    const moved = { ...a, status: "in-review", updatedAt: iso(30_000) };
    const deliveries = kaneoChangeDeliveries(prev, [moved, b]);
    expect(deliveries.map((d) => d.kind)).toEqual(["kaneo-change", "kaneo-change"]);
    expect(deliveries[0]?.summary).toContain("in-progress -> in-review");
    expect(deliveries[0]?.link).toContain("kaneo.security.cab");
    expect(deliveries[1]?.summary).toContain("appeared");
    expect(kaneoChangeDeliveries(new Map([[a.id, a]]), [a])).toEqual([]);
  });
});

describe("codex findings on PR #5141 (#3321)", () => {
  test("watchdog stabilizer pins an episode to its first-observed since until it clears", () => {
    const stab = new WatchdogEpisodeStabilizer();
    const first = stab.stabilize([watchdogItem("watchdog:fleet-feed", 300_000)]);
    // A later derivation of the SAME breach carries a different
    // (now-based) since; the delivery id must not move.
    const laterItem = { ...watchdogItem("watchdog:fleet-feed", 60_000) };
    const second = stab.stabilize([laterItem]);
    expect(second[0]?.since).toBe(first[0]?.since);
    expect(watchdogDeliveries(second)[0]?.sourceId).toBe(watchdogDeliveries(first)[0]?.sourceId);
    // The episode clears; a genuinely new episode gets its own id.
    stab.stabilize([]);
    const third = stab.stabilize([laterItem]);
    expect(third[0]?.since).toBe(laterItem.since);
  });

  test("undated kaneo transitions get per-transition ids; a re-diff keeps its id", () => {
    const undated = (status: string, seq: number): KaneoCardSnapshot => ({
      id: "kaneo:p/t1",
      status,
      title: "Card t1",
      seq,
    });
    // in-progress (seq 0) -> in-review: first undated transition.
    const d1 = kaneoChangeDeliveries(new Map([["kaneo:p/t1", undated("in-progress", 0)]]), [undated("in-review", 0)]);
    // Re-diffing the same observed transition (retry after a failed
    // append, baseline not advanced) reproduces the same id.
    const retry = kaneoChangeDeliveries(new Map([["kaneo:p/t1", undated("in-progress", 0)]]), [
      undated("in-review", 0),
    ]);
    expect(retry[0]?.sourceId).toBe(d1[0]?.sourceId);
    // The cycle continues: in-review (seq 1) -> in-progress -> in-review.
    const d2 = kaneoChangeDeliveries(new Map([["kaneo:p/t1", undated("in-review", 1)]]), [undated("in-progress", 1)]);
    const d3 = kaneoChangeDeliveries(new Map([["kaneo:p/t1", undated("in-progress", 2)]]), [undated("in-review", 2)]);
    expect(d2[0]?.sourceId).not.toBe(d1[0]?.sourceId);
    // Same from->to as d1, but a distinct real transition: new id.
    expect(d3[0]?.sourceId).not.toBe(d1[0]?.sourceId);
    expect(d1[0]?.sourceId).toContain("undated:1");
    expect(d3[0]?.sourceId).toContain("undated:3");
  });

  test("concurrent ingestion of the same delivery ingests it exactly once", async () => {
    const { events, store } = tmpStores();
    const deliveries = watchdogDeliveries([watchdogItem("watchdog:registry", 120_000)]);
    let records = 0;
    const slow = {
      record: async (e: Parameters<EventsStore["record"]>[0]) => {
        records++;
        await new Promise((r) => setTimeout(r, 25));
        return events.record(e);
      },
    };
    const [a, b] = await Promise.all([store.ingest(slow, deliveries), store.ingest(slow, deliveries)]);
    expect(records).toBe(1);
    expect(a.ingested + b.ingested).toBe(1);
    expect(a.duplicates + b.duplicates).toBe(1);
    expect(await events.list()).toHaveLength(1);
  });

  test("ingestActivity reports failed appends so callers can retain deliveries", async () => {
    const { store } = tmpStores();
    const deliveries = watchdogDeliveries([watchdogItem("watchdog:registry", 60_000)]);
    const failing = {
      record: async () => {
        throw new Error("disk full");
      },
    };
    const result = await ingestActivity(store, failing, deliveries);
    expect(result.failed).toBe(1);
    expect(result.ingested).toBe(0);
  });
});

describe("undated seq seeding (#3321, PR #5141 codex round 2)", () => {
  test("maxUndatedSeqByCard parses appeared and status ids, ignores dated ones", () => {
    const m = maxUndatedSeqByCard([
      "card-a:appeared:undated:2",
      "card-a:status:in-progress->in-review:undated:5",
      "card-a:status:in-review->in-progress:undated:3",
      "card-b:status:in-progress->in-review:2026-11-03T00:00:00.000Z",
      "watchdog:episode-1",
    ]);
    expect(m.get("card-a")).toBe(5);
    expect(m.has("card-b")).toBe(false);
    expect(m.size).toBe(1);
  });

  test("seenIds returns the persisted set per source after a restart", async () => {
    const { activityFile, events, store } = tmpStores();
    const delivery: ActivityDelivery = {
      source: "kaneo",
      sourceId: "card-a:status:in-progress->in-review:undated:1",
      at: iso(0),
      kind: "kaneo-change",
      ref: "card-a",
      summary: "moved",
    };
    await ingestActivity(store, events, [delivery]);
    const restarted = new IngestionStore(activityFile);
    expect(await restarted.seenIds("kaneo")).toEqual([delivery.sourceId]);
    expect(await restarted.seenIds("watchdog")).toEqual([]);
    expect(maxUndatedSeqByCard(await restarted.seenIds("kaneo")).get("card-a")).toBe(1);
  });
});

describe("ingestion store (#3321)", () => {
  test("duplicate delivery replay is a no-op; checkpoint counts both", async () => {
    const { events, store } = tmpStores();
    const deliveries = watchdogDeliveries([watchdogItem("watchdog:fleet-feed", 120_000)]);
    const first = await store.ingest(events, deliveries);
    expect(first).toEqual({ ingested: 1, duplicates: 0, failed: 0, failedIds: [] });
    const second = await store.ingest(events, deliveries);
    expect(second).toEqual({ ingested: 0, duplicates: 1, failed: 0, failedIds: [] });
    const stream = await events.list();
    expect(stream).toHaveLength(1);
    expect(stream[0]?.kind).toBe("watchdog-event");
    const cp = await store.checkpoint("watchdog");
    expect(cp?.deliveries).toBe(1);
    expect(cp?.duplicates).toBe(1);
    expect(cp?.lastSourceId).toBe(deliveries[0]?.sourceId);
  });

  test("restart resumes from the persisted checkpoint: replay stays duplicate", async () => {
    const { eventsFile, activityFile, events, store } = tmpStores();
    const deliveries: ActivityDelivery[] = [
      {
        source: "kaneo",
        sourceId: "kaneo:p/t1:appeared:stamp-1",
        at: iso(90_000),
        kind: "kaneo-change",
        ref: "kaneo:p/t1",
        summary: "Kaneo card appeared in in-progress: Card t1",
        link: "https://kaneo.security.cab/x",
      },
    ];
    await store.ingest(events, deliveries);
    // Fresh store instances over the same files = a process restart.
    const store2 = new IngestionStore(activityFile);
    const events2 = new EventsStore(eventsFile);
    const replay = await store2.ingest(events2, deliveries);
    expect(replay).toEqual({ ingested: 0, duplicates: 1, failed: 0, failedIds: [] });
    expect(await events2.list()).toHaveLength(1);
    const cp = await store2.checkpoint("kaneo");
    expect(cp?.deliveries).toBe(1);
    expect(cp?.lastSourceId).toBe("kaneo:p/t1:appeared:stamp-1");
  });

  test("a failed stream append is not marked seen, so a replay retries it", async () => {
    const { events, store } = tmpStores();
    const deliveries = watchdogDeliveries([watchdogItem("watchdog:registry", 60_000)]);
    const failing = {
      record: async () => {
        throw new Error("disk full");
      },
    };
    const first = await store.ingest(failing, deliveries);
    expect(first).toEqual({ ingested: 0, duplicates: 0, failed: 1, failedIds: [deliveries[0]!.sourceId] });
    const retry = await store.ingest(events, deliveries);
    expect(retry).toEqual({ ingested: 1, duplicates: 0, failed: 0, failedIds: [] });
  });

  test("gap lifecycle end-to-end: outage opens a rendered hole, backfill flips it", async () => {
    const { store } = tmpStores();
    const from = iso(3_600_000);
    const to = iso(1_800_000);
    const gap = await store.recordGap("kaneo", from, to);
    expect(gap.status).toBe("open");
    // A continuing outage extends the SAME open gap, never a second one.
    const extended = await store.recordGap("kaneo", from, iso(900_000));
    expect(extended.id).toBe(gap.id);
    expect((await store.gaps()).filter((g) => g.source === "kaneo")).toHaveLength(1);
    let html = gapsHtml(await store.gaps());
    expect(html).toContain("KNOWN HOLES");
    expect(html).toContain("open");
    // Backfill lands -> status flips, and the render says so.
    const resolved = await store.resolveGap(gap.id, "backfilled");
    expect(resolved?.status).toBe("backfilled");
    html = gapsHtml(await store.gaps());
    expect(html).toContain("backfilled");
  });

  test("resolveGap extends the gap window to the recovery time (PR #5141 codex)", async () => {
    const { store } = tmpStores();
    const gap = await store.recordGap("kaneo", iso(3_600_000), iso(1_800_000));
    const recoveredAt = iso(60_000);
    const resolved = await store.resolveGap(gap.id, "backfilled", recoveredAt);
    expect(resolved?.status).toBe("backfilled");
    // The outage ran until recovery, not until the last failed poll:
    // the resolved window ends at the recovery moment.
    expect(resolved?.to).toBe(recoveredAt);
    // An earlier `to` never shrinks the window.
    const gap2End = iso(900_000);
    const gap2 = await store.recordGap("watchdog:x", iso(3_600_000), gap2End);
    const resolved2 = await store.resolveGap(gap2.id, "backfilled", iso(1_800_000));
    expect(resolved2?.to).toBe(gap2End);
  });

  test("watchdog gap sync: breach opens a gap, recovery resolves it as backfilled", async () => {
    const { store } = tmpStores();
    const item = watchdogItem("watchdog:fleet-feed", 600_000);
    await syncWatchdogGaps(store, [item], new Date());
    let gaps = await store.gaps();
    expect(gaps).toHaveLength(1);
    expect(gaps[0]?.source).toBe("watchdog:fleet-feed");
    expect(gaps[0]?.status).toBe("open");
    expect(gaps[0]?.from).toBe(item.since);
    await syncWatchdogGaps(store, [], new Date());
    gaps = await store.gaps();
    expect(gaps[0]?.status).toBe("backfilled");
    expect(gaps[0]?.resolvedAt).toBeTruthy();
  });

  test("watchdog gap sync queue: an older overlapping pass cannot resurrect a gap the newest pass resolved (PR #5141 codex round 5)", async () => {
    const { store } = tmpStores();
    const itemA = watchdogItem("watchdog:a", 600_000);
    const itemB = watchdogItem("watchdog:b", 600_000);
    // Gate the older pass mid-reconciliation: its second recordGap
    // waits until the test releases it. Without the queue, the
    // newer healthy pass would run inside that window — resolving
    // A and finishing — and the older pass would then create B,
    // leaving an outage the newest observation says is over still
    // reported open.
    let releaseB: () => void = () => {};
    const gateB = new Promise<void>((resolve) => {
      releaseB = resolve;
    });
    let recordCalls = 0;
    const realRecord = store.recordGap.bind(store);
    store.recordGap = async (source: string, from: string, to: string) => {
      recordCalls += 1;
      if (recordCalls === 2) await gateB;
      return realRecord(source, from, to);
    };
    const enqueue = createWatchdogGapSyncQueue();
    const older = enqueue(store, [itemA, itemB], new Date());
    const newer = enqueue(store, [], new Date());
    // Wait until the older pass is provably parked on B's record,
    // then release it; the newer pass may only run after it.
    for (let i = 0; i < 200 && recordCalls < 2; i += 1) {
      await new Promise((r) => setTimeout(r, 10));
    }
    expect(recordCalls).toBe(2);
    releaseB();
    await Promise.all([older, newer]);
    const gaps = await store.gaps();
    expect(gaps.map((g) => g.source).sort()).toEqual(["watchdog:a", "watchdog:b"]);
    expect(gaps.filter((g) => g.status === "open")).toHaveLength(0);
    expect(gaps.every((g) => g.status === "backfilled")).toBe(true);
  });

  test("coverage documents github as unsupported pending #2072", () => {
    const github = ACTIVITY_SOURCE_COVERAGE.find((c) => c.source === "github");
    expect(github?.status).toBe("unsupported");
    expect(github?.unsupported).toContain("#2072");
    const html = activityCoverageHtml();
    expect(html).toContain("watchdog");
    expect(html).toContain("kaneo");
    expect(html).toContain("unsupported");
  });
});

describe("ingestion store load validation (#5232)", () => {
  const invalidRecords: Array<{
    description: string;
    record: "gap" | "checkpoint";
    patch: Record<string, unknown>;
  }> = [
    { description: "impossible gap start", record: "gap", patch: { from: "2026-02-30T00:00:00.000Z" } },
    { description: "impossible gap end", record: "gap", patch: { to: "2026-04-31T00:00:00.000Z" } },
    { description: "non-ISO gap detection date", record: "gap", patch: { detectedAt: "March 2, 2026" } },
    { description: "garbage gap date", record: "gap", patch: { from: "not-a-date" } },
    {
      description: "impossible gap resolution date",
      record: "gap",
      patch: { resolvedAt: "2026-02-30T00:00:00.000Z" },
    },
    {
      description: "impossible checkpoint last-event date",
      record: "checkpoint",
      patch: { lastEventAt: "2026-02-30T00:00:00.000Z" },
    },
    {
      description: "non-ISO checkpoint update date",
      record: "checkpoint",
      patch: { updatedAt: "March 2, 2026" },
    },
    { description: "garbage checkpoint date", record: "checkpoint", patch: { updatedAt: "not-a-date" } },
    { description: "negative deliveries count", record: "checkpoint", patch: { deliveries: -1 } },
    { description: "negative duplicates count", record: "checkpoint", patch: { duplicates: -2 } },
  ];

  test.each(invalidRecords)("quarantines $description and starts empty", async ({ record, patch }) => {
    const dir = mkdtempSync(join(tmpdir(), "agg-activity-invalid-"));
    const file = join(dir, "activity.json");
    const persisted = validIngestionFile();
    Object.assign(record === "gap" ? persisted.gaps[0]! : persisted.checkpoints.kaneo, patch);
    const raw = JSON.stringify(persisted);
    writeFileSync(file, raw);

    const errors: string[] = [];
    const origError = console.error;
    console.error = (...args: unknown[]) => {
      errors.push(args.map(String).join(" "));
    };
    try {
      const store = new IngestionStore(file);
      expect(await store.checkpoints()).toEqual([]);
      expect(await store.seenIds("kaneo")).toEqual([]);
      expect(await store.gaps()).toEqual([]);
    } finally {
      console.error = origError;
    }

    expect(
      errors.some((message) => message.includes("is unparseable") && message.includes("quarantined the original bytes")),
    ).toBe(true);
    const quarantined = readdirSync(dir).filter((name) => name.startsWith("activity.json.corrupt-"));
    expect(quarantined).toHaveLength(1);
    expect(readFileSync(join(dir, quarantined[0]!), "utf8")).toBe(raw);
    expect(existsSync(file)).toBe(false);
  });

  test("valid ISO dates and counts load unchanged without quarantine", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-activity-valid-"));
    const file = join(dir, "activity.json");
    const persisted = validIngestionFile();
    const raw = JSON.stringify(persisted);
    writeFileSync(file, raw);

    const store = new IngestionStore(file);
    expect(await store.checkpoints()).toEqual([persisted.checkpoints.kaneo, persisted.checkpoints.watchdog]);
    expect(await store.seenIds("kaneo")).toEqual(["kaneo:card-1"]);
    expect(await store.gaps()).toEqual(persisted.gaps);
    expect(readFileSync(file, "utf8")).toBe(raw);
    expect(readdirSync(dir).filter((name) => name.startsWith("activity.json.corrupt-"))).toEqual([]);
  });
});

describe("activity summary caps never split a surrogate pair (#7725)", () => {
  const roundTrips = (s: string) => new TextDecoder().decode(new TextEncoder().encode(s)) === s;

  test("watchdog writer drops an astral character straddling the 500 cap whole", () => {
    const [d] = watchdogDeliveries([watchdogItem("watchdog:emoji", 60_000, "x".repeat(499) + "\u{1F600}" + "tail")]);
    expect(d?.summary).toHaveLength(499);
    expect(d?.summary.endsWith("x")).toBe(true);
    expect(roundTrips(d?.summary ?? "")).toBe(true);
  });

  test("watchdog writer keeps an astral character fully inside the cap intact", () => {
    const [d] = watchdogDeliveries([watchdogItem("watchdog:emoji-fit", 60_000, "x".repeat(498) + "\u{1F600}")]);
    expect(d?.summary).toHaveLength(500);
    expect(d?.summary.endsWith("\u{1F600}")).toBe(true);
    expect(roundTrips(d?.summary ?? "")).toBe(true);
  });

  test("kaneo writer drops an astral character straddling the composed 500 cap whole", () => {
    const prefix = "Kaneo card appeared in open: ";
    const card: KaneoCardSnapshot = {
      id: "kaneo:p/emoji",
      status: "open",
      title: "y".repeat(500 - prefix.length - 1) + "\u{1F600}" + "tail",
      updatedAt: iso(60_000),
    };
    const [d] = kaneoChangeDeliveries(new Map(), [card]);
    expect(d?.summary).toHaveLength(499);
    expect(d?.summary.endsWith("y")).toBe(true);
    expect(roundTrips(d?.summary ?? "")).toBe(true);
  });

  test("record-time cap drops an astral character the link suffix pushes across 500", async () => {
    const { events, store } = tmpStores();
    const delivery: ActivityDelivery = {
      source: "kaneo",
      sourceId: "kaneo:p/emoji:appeared:undated:1",
      at: iso(0),
      kind: "kaneo-change",
      ref: "kaneo:p/emoji",
      summary: "s".repeat(494),
      link: "ab\u{1F600}cd",
    };
    const result = await store.ingest(events, [delivery]);
    expect(result.ingested).toBe(1);
    const stream = await events.list();
    expect(stream).toHaveLength(1);
    expect(stream[0]?.summary).toHaveLength(499);
    expect(stream[0]?.summary.endsWith("b")).toBe(true);
    expect(roundTrips(stream[0]?.summary ?? "")).toBe(true);
  });
});
