// #3321: activity ingestion — watchdog-event and Kaneo-change writers
// into the #1739 stream, with per-source persisted checkpoints,
// duplicate-delivery dedupe, restart resume, and gap records that a
// backfill flips. GitHub stays unsupported pending #2072 (asserted in
// the coverage table so the gap is documented, not implied away).
import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  ACTIVITY_SOURCE_COVERAGE,
  IngestionStore,
  activityCoverageHtml,
  gapsHtml,
  kaneoChangeDeliveries,
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

describe("ingestion store (#3321)", () => {
  test("duplicate delivery replay is a no-op; checkpoint counts both", async () => {
    const { events, store } = tmpStores();
    const deliveries = watchdogDeliveries([watchdogItem("watchdog:fleet-feed", 120_000)]);
    const first = await store.ingest(events, deliveries);
    expect(first).toEqual({ ingested: 1, duplicates: 0, failed: 0 });
    const second = await store.ingest(events, deliveries);
    expect(second).toEqual({ ingested: 0, duplicates: 1, failed: 0 });
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
    expect(replay).toEqual({ ingested: 0, duplicates: 1, failed: 0 });
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
    expect(first).toEqual({ ingested: 0, duplicates: 0, failed: 1 });
    const retry = await store.ingest(events, deliveries);
    expect(retry).toEqual({ ingested: 1, duplicates: 0, failed: 0 });
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
