import { describe, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { FreshnessStore, type FreshnessInput, type FreshnessNotice } from "../src/freshness.ts";
import {
  fleetFeedRevisionReader,
  FreshnessReconciler,
  RECONCILE_INTERVAL_MS,
  type CanonicalRevision,
  type ReconcileJournal,
} from "../src/freshness-reconcile.ts";
import { loadConfig } from "../src/config.ts";
import { buildHealth, createProbeCache, type HealthInputs } from "../src/health.ts";
import type { FleetFeed } from "../src/feed.ts";
import { renderHealth } from "../src/render.ts";

// #3976 acceptance scenarios: a dropped event repaired by
// reconciliation, producer outage/recovery, event journal loss/reset,
// quiet-but-healthy sources, and stale data remaining visibly stale.
// Oracle: the journal's own contents (a notice exists under the
// revision's eventId, or it does not) and the health view's fields —
// never the reconciler's word alone.

const T0 = Date.parse("2026-11-03T12:00:00Z");
const at = (ms: number) => new Date(T0 + ms);

function feedRevision(occurredMs: number): CanonicalRevision {
  return {
    source: "fleet-feed",
    eventId: `fleet-feed:${at(occurredMs).toISOString()}:feedhash`,
    scope: ["feed"],
    occurredAt: at(occurredMs).toISOString(),
  };
}

/** A journal with FreshnessStore semantics (dedupe by eventId,
 *  monotonic sequences) whose cursor can be reset mid-test. */
class FakeJournal implements ReconcileJournal {
  events: FreshnessNotice[] = [];
  cursor = 0;
  pendingNext = false;
  failReads = false;
  publishCalls: FreshnessInput[] = [];

  async listAfter(after: number) {
    if (this.failReads) throw new Error("journal io error");
    return {
      cursor: this.cursor,
      resetRequired: after > this.cursor,
      events: this.events.filter((e) => e.sequence > after).map((e) => ({ ...e, scope: [...e.scope] })),
    };
  }

  async publish(input: FreshnessInput) {
    this.publishCalls.push(input);
    const existing = this.events.find((e) => e.eventId === input.eventId);
    if (existing) return { notice: existing, duplicate: true, pending: false };
    if (this.pendingNext) {
      this.pendingNext = false;
      return {
        notice: { ...input, schemaVersion: 1 as const, sequence: 0, observedAt: at(0).toISOString() },
        duplicate: false,
        pending: true,
      };
    }
    const notice: FreshnessNotice = {
      ...input,
      schemaVersion: 1,
      sequence: ++this.cursor,
      observedAt: at(0).toISOString(),
    };
    this.events.push(notice);
    return { notice, duplicate: false, pending: false };
  }

  reset() {
    this.events = [];
    this.cursor = 0;
  }
}

const readerOf = (revision: CanonicalRevision | null) => ({
  source: "fleet-feed" as const,
  read: async () => revision,
});

describe("#3976 freshness reconciliation", () => {
  test("dropped event repaired by reconciliation (real FreshnessStore)", async () => {
    const dir = await mkdtemp(join(tmpdir(), "freshness-reconcile-3976-"));
    try {
      const store = new FreshnessStore(join(dir, "freshness.json"));
      const revision = feedRevision(0);
      const reconciler = new FreshnessReconciler(store, [readerOf(revision)]);

      const view = await reconciler.reconcile(at(60_000));
      // The synthetic invalidation is in the journal, under the
      // revision's own eventId, reasoned "reconcile".
      const page = await store.listAfter(0);
      expect(page.events).toHaveLength(1);
      expect(page.events[0]!.eventId).toBe(revision.eventId);
      expect(page.events[0]!.reason).toBe("reconcile");
      expect(view.gaps).toMatchObject({ open: 0, repairedTotal: 1, resetsTotal: 0 });
      expect(view.state).toBe("up");
      expect(view.lastSuccessfulReconcileAt).toBe(at(60_000).toISOString());

      // A second pass finds the notice and repairs nothing further.
      const again = await reconciler.reconcile(at(120_000));
      expect(again.gaps.repairedTotal).toBe(1);
      expect((await store.listAfter(0)).events).toHaveLength(1);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("producer outage is reported unavailable, recovery repairs and clears it", async () => {
    const journal = new FakeJournal();
    let down = true;
    const reader = {
      source: "fleet-feed" as const,
      read: async (): Promise<CanonicalRevision | null> => {
        if (down) throw new Error("collector unreachable");
        return feedRevision(0);
      },
    };
    const reconciler = new FreshnessReconciler(journal, [reader]);

    const outage = await reconciler.reconcile(at(0));
    const feedRow = outage.sources.find((s) => s.source === "fleet-feed")!;
    expect(feedRow.state).toBe("unavailable");
    expect(feedRow.missingNotice).toBe(false); // nothing invented during the outage
    expect(journal.publishCalls).toHaveLength(0);
    expect(outage.state).toBe("degraded");

    down = false;
    const recovered = await reconciler.reconcile(at(60_000));
    const row = recovered.sources.find((s) => s.source === "fleet-feed")!;
    expect(row.state).toBe("live");
    expect(row.missingNotice).toBe(false);
    expect(recovered.gaps.repairedTotal).toBe(1);
    expect(recovered.state).toBe("up");
  });

  test("event journal loss/reset is counted, not silently re-baselined", async () => {
    const journal = new FakeJournal();
    // Two real source-event notices give the journal a cursor of 2.
    await journal.publish({
      eventId: feedRevision(0).eventId,
      source: "fleet-feed",
      scope: ["feed"],
      occurredAt: at(0).toISOString(),
      reason: "source-event",
    });
    const reconciler = new FreshnessReconciler(journal, [readerOf(feedRevision(0))]);
    const before = await reconciler.reconcile(at(60_000));
    expect(before.gaps.resetsTotal).toBe(0);
    expect(before.cursor).toBe(1);

    journal.reset(); // the journal file is lost/replaced
    const after = await reconciler.reconcile(at(120_000));
    expect(after.gaps.resetsTotal).toBe(1);
    expect(after.gaps.lastResetAt).toBe(at(120_000).toISOString());
    // The revision's notice died with the journal: reconciliation
    // repairs it again in the new journal.
    expect(journal.events.map((e) => e.eventId)).toContain(feedRevision(0).eventId);
  });

  test("quiet-but-healthy source is silent, not an outage", async () => {
    const journal = new FakeJournal();
    const reconciler = new FreshnessReconciler(journal, [readerOf(null)]);
    const view = await reconciler.reconcile(at(0));
    const row = view.sources.find((s) => s.source === "fleet-feed")!;
    expect(row.state).toBe("silent");
    expect(row.lastEventAt).toBeNull();
    expect(view.gaps.open).toBe(0);
    // Push machinery is healthy even though the source said nothing:
    // connectivity and source freshness are separate facts.
    expect(view.state).toBe("up");
  });

  test("stale data remains visibly stale while push stays up", async () => {
    const journal = new FakeJournal();
    const revision = feedRevision(-30 * 60_000);
    await journal.publish({
      eventId: revision.eventId,
      source: "fleet-feed",
      scope: ["feed"],
      occurredAt: revision.occurredAt,
      reason: "source-event",
    });
    // Age the retained notice: its observedAt is 30 minutes old.
    journal.events[0] = { ...journal.events[0]!, observedAt: at(-30 * 60_000).toISOString() };
    const reconciler = new FreshnessReconciler(journal, [readerOf(revision)]);
    const view = await reconciler.reconcile(at(0));
    const row = view.sources.find((s) => s.source === "fleet-feed")!;
    expect(row.state).toBe("stale");
    expect(row.ageMs).toBe(30 * 60_000);
    expect(row.missingNotice).toBe(false);
    expect(view.lastEventAt).toBe(at(-30 * 60_000).toISOString());
    expect(view.state).toBe("up"); // the machinery is fine; the DATA is stale, and says so
  });

  test("a non-durable synthetic notice leaves the gap open and the reconcile unsuccessful", async () => {
    const journal = new FakeJournal();
    journal.pendingNext = true;
    const reconciler = new FreshnessReconciler(journal, [readerOf(feedRevision(0))]);
    const view = await reconciler.reconcile(at(0));
    expect(view.gaps.open).toBe(1);
    expect(view.gaps.repairedTotal).toBe(0);
    expect(view.state).toBe("degraded");
    expect(view.lastSuccessfulReconcileAt).toBeNull();
    const row = view.sources.find((s) => s.source === "fleet-feed")!;
    expect(row.missingNotice).toBe(true);
    expect(row.lagMs).toBe(0); // occurredAt == T0, now == T0: lag starts at 0 and grows while open
  });

  test("journal unreadable is push down, never a healthy guess", async () => {
    const journal = new FakeJournal();
    journal.failReads = true;
    const reconciler = new FreshnessReconciler(journal, [readerOf(feedRevision(0))]);
    const view = await reconciler.reconcile(at(0));
    expect(view.state).toBe("down");
    expect(view.detail).toContain("journal unreadable");
  });

  test("reconcile interval is the 5-minute design cadence", () => {
    expect(RECONCILE_INTERVAL_MS).toBe(5 * 60_000);
  });
});

describe("#3976 fleet-feed canonical reader", () => {
  test("recomputes the collector's notice identity from the feed file", async () => {
    const dir = await mkdtemp(join(tmpdir(), "freshness-feed-reader-3976-"));
    try {
      const file = join(dir, "fleet-feed.json");
      const contents = JSON.stringify({ version: 1, generatedAt: "2026-11-03T12:00:00Z", quota: {} });
      await writeFile(file, contents);
      const revision = await fleetFeedRevisionReader(file).read();
      const sha256 = createHash("sha256").update(contents, "utf8").digest("hex");
      expect(revision?.eventId).toBe(`fleet-feed:2026-11-03T12:00:00Z:${sha256}`);
      expect(revision?.scope).toEqual(["feed"]);
      expect(revision?.occurredAt).toBe("2026-11-03T12:00:00.000Z");
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("an invalid feed file throws (source unavailable), never a fabricated revision", async () => {
    const dir = await mkdtemp(join(tmpdir(), "freshness-feed-reader-3976-"));
    try {
      const file = join(dir, "fleet-feed.json");
      await writeFile(file, "not json");
      await expect(fleetFeedRevisionReader(file).read()).rejects.toThrow();
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});

describe("#3976 push health on the health surfaces", () => {
  const NOW = Date.parse("2026-11-03T12:00:00Z");
  const cfg = loadConfig({ PM_AGG_DEV: "1" });
  const feed = {
    version: 1,
    generatedAt: new Date(NOW - 60_000).toISOString(),
    quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
    attention: [],
    registry: [],
    projects: [],
  } as unknown as FleetFeed;

  async function viewWith(pushHealth: HealthInputs["pushHealth"]) {
    return buildHealth(
      {
        mode: "live",
        cfg,
        feed,
        feedFileMtimeMs: NOW - 60_000,
        pushHealth,
      },
      {
        fetcher: async () => ({ status: 200 }),
        cache: createProbeCache(),
        now: NOW,
        rfaState: async () => ({ repositories: [], backends: [], workItems: [] }),
      },
    );
  }

  test("/api/health view carries push state, last event, last reconcile, and gaps", async () => {
    const journal = new FakeJournal();
    const reconciler = new FreshnessReconciler(journal, [readerOf(feedRevision(0))]);
    const push = await reconciler.reconcile(at(0));
    const view = await viewWith(push);
    expect(view.push?.state).toBe("up");
    expect(view.push?.lastSuccessfulReconcileAt).toBe(at(0).toISOString());
    expect(view.push?.gaps).toMatchObject({ open: 0, repairedTotal: 1, resetsTotal: 0 });
    expect(view.push?.sources.find((s) => s.source === "fleet-feed")?.state).toBe("live");
  });

  test("push down degrades the health view; no push input stays null, never invented", async () => {
    const journal = new FakeJournal();
    journal.failReads = true;
    const reconciler = new FreshnessReconciler(journal, [readerOf(null)]);
    const down = await reconciler.reconcile(at(0));
    const view = await viewWith(down);
    expect(view.push?.state).toBe("down");
    expect(view.degraded).toBe(true);

    const bare = await viewWith(undefined);
    expect(bare.push).toBeNull();
  });

  test("/health renders the push section with per-source rows and gaps", async () => {
    const journal = new FakeJournal();
    const reconciler = new FreshnessReconciler(journal, [readerOf(feedRevision(0))]);
    const push = await reconciler.reconcile(at(0));
    const view = await viewWith(push);
    const html = renderHealth(view, {
      board: "live",
      roadmap: "live",
      items: "live",
      comments: "live",
      quota: "live",
      usage: "live",
      attention: "live",
      orchestration: "live",
      dns: "live",
      registry: "live",
      aa: "live",
    });
    expect(html).toContain('id="push-health"');
    expect(html).toContain("push:");
    expect(html).toContain("fleet-feed");
    expect(html).toContain("last successful reconcile");
  });
});
