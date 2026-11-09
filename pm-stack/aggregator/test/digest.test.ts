import { describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  buildDigest,
  digestText,
  listDigestArchives,
  readDigestArchive,
  writeDigestArchive,
  type DigestInputs,
} from "../src/digest.ts";
import type { NeedsDrewCensus } from "../src/needs-drew.ts";
import type { ProjectPoolItem, ProvenanceDetail, QuotaView } from "../src/model.ts";

// Fixture tests for the scheduled digest (#1278): sections are built
// from the canonical projection only, every item is linked, the header
// states freshness, and a missing/stale source degrades its section
// visibly instead of omitting it.

const NOW = new Date("2026-11-02T13:00:00Z");

function item(partial: Partial<ProjectPoolItem> & { number: number }): ProjectPoolItem {
  return {
    id: `PVTI_${partial.number}`,
    url: `https://github.com/orgs/developing-today/projects/4?pane=issue&itemId=${partial.number}`,
    projectUrl: "https://github.com/orgs/developing-today/projects/4",
    ref: `developing-today/code#${partial.number}`,
    repo: "developing-today/code",
    issueUrl: `https://github.com/developing-today/code/issues/${partial.number}`,
    title: `item ${partial.number}`,
    state: "open",
    contentKind: "issue",
    status: null,
    area: null,
    horizon: null,
    kind: "Task",
    updatedAt: "2026-11-02T10:00:00Z",
    ...partial,
  };
}

const census: NeedsDrewCensus = {
  fetchedAt: "2026-11-02T12:30:00Z",
  complete: true,
  sources: [],
  items: [
    {
      ref: "developing-today/code#42",
      repo: "developing-today/code",
      number: 42,
      kind: "issue",
      title: "decide the thing",
      url: "https://github.com/developing-today/code/issues/42",
      labels: ["user/escalated"],
      needsLabels: ["user/escalated"],
      reason: "needs a budget call",
      updatedAt: "2026-11-02T09:00:00Z",
    },
  ],
};

const quota: QuotaView = {
  pools: [
    {
      id: "pool-ok",
      provider: "Fine",
      plan: "Plan",
      state: "healthy",
      windows: [
        { name: "weekly", unit: "tokens", limit: 1000, used: 100, resetsAt: "2026-11-09T00:00:00Z", percent: 10 },
      ],
    },
    {
      id: "pool-hot",
      provider: "Hot",
      plan: "Plan",
      state: "near",
      windows: [
        { name: "weekly", unit: "tokens", limit: 1000, used: 950, resetsAt: "2026-11-09T00:00:00Z", percent: 95 },
      ],
    },
  ],
  burn: [],
  choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] },
};

function inputs(partial: Partial<DigestInputs> = {}): DigestInputs {
  return {
    pool: [
      item({ number: 1, status: "In Progress" }),
      item({ number: 2, state: "merged", contentKind: "pr", updatedAt: "2026-11-02T08:00:00Z" }),
      item({ number: 3, blockedBy: ["developing-today/code#1"] }),
      item({ number: 4, updatedAt: "2026-10-20T08:00:00Z" }), // outside the daily window
      item({ number: 5, state: "closed", updatedAt: "2026-10-28T08:00:00Z" }), // done 5d ago: weekly news, not daily
    ],
    needsDrew: census,
    quota,
    provenanceDetail: null,
    now: NOW,
    kind: "daily",
    ...partial,
  };
}

function section(d: ReturnType<typeof buildDigest>, id: string) {
  const s = d.sections.find((x) => x.id === id);
  if (!s) throw new Error(`no section ${id}`);
  return s;
}

describe("buildDigest", () => {
  test("sections come from the projection, windowed, with links", () => {
    const d = buildDigest(inputs());
    expect(d.date).toBe("2026-11-02");
    expect(d.archiveRoute).toBe("/digest?date=2026-11-02");
    expect(section(d, "moved").items.map((i) => i.ref)).toEqual(["developing-today/code#1", "developing-today/code#3"]);
    expect(section(d, "done").items.map((i) => i.ref)).toEqual(["developing-today/code#2"]);
    const blocked = section(d, "blocked");
    expect(blocked.items).toHaveLength(1);
    expect(blocked.items[0].note).toContain("blocked by developing-today/code#1");
    expect(blocked.items[0].url).toBe("https://github.com/developing-today/code/issues/3");
    const needs = section(d, "needsDrew");
    expect(needs.stale).toBe(false);
    expect(needs.items[0].note).toBe("needs a budget call");
    const outlook = section(d, "quota");
    expect(outlook.items.map((i) => i.ref)).toEqual(["pool-hot"]);
    expect(outlook.items[0].note).toContain("near wall");
  });

  test("weekly window reaches back seven days", () => {
    const d = buildDigest(inputs({ kind: "weekly" }));
    expect(d.windowHours).toBe(168);
    expect(d.archiveRoute).toBe("/digest?date=2026-11-02&kind=weekly");
    expect(section(d, "done").items.map((i) => i.ref)).toContain("developing-today/code#5");
  });

  test("an unavailable pool degrades the pool sections visibly", () => {
    const d = buildDigest(inputs({ poolAvailable: false }));
    expect(section(d, "moved").stale).toBe(true);
    expect(section(d, "moved").staleNote).toContain("no projects section");
  });

  test("an unprimed (fixtures) facet in live mode is stale, not healthy", () => {
    const detail = {
      items: { text: "fixtures", state: "fixtures", ageMs: 0 },
    } as unknown as ProvenanceDetail;
    const live = buildDigest(inputs({ provenanceDetail: detail, sourceKind: "live" }));
    expect(section(live, "moved").stale).toBe(true);
    const fixtures = buildDigest(inputs({ provenanceDetail: detail, sourceKind: "fixtures" }));
    expect(section(fixtures, "moved").stale).toBe(false);
  });

  test("a missing census degrades Needs Drew visibly, never silently", () => {
    const d = buildDigest(inputs({ needsDrew: null }));
    const needs = section(d, "needsDrew");
    expect(needs.stale).toBe(true);
    expect(needs.staleNote).toContain("unavailable");
    expect(needs.items).toHaveLength(0);
    expect(digestText(d)).toContain("STALE");
  });

  test("an incomplete or old census is marked stale with its age", () => {
    const d = buildDigest(inputs({ needsDrew: { ...census, complete: false, fetchedAt: "2026-11-01T00:00:00Z" } }));
    const needs = section(d, "needsDrew");
    expect(needs.stale).toBe(true);
    expect(needs.staleNote).toContain("incomplete");
  });

  test("a stale pool facet marks the pool sections stale", () => {
    const detail = {
      items: { text: "feed 5h old", state: "stale", ageMs: 5 * 3600000 },
    } as unknown as ProvenanceDetail;
    const d = buildDigest(inputs({ provenanceDetail: detail }));
    expect(section(d, "moved").stale).toBe(true);
    expect(section(d, "moved").staleNote).toContain("items");
    expect(d.freshness).toContainEqual({ facet: "items", state: "stale", ageMs: 5 * 3600000 });
  });

  test("quota wall prediction fires when burn fills the window before reset", () => {
    const burning: QuotaView = {
      ...quota,
      pools: [
        {
          id: "pool-burn",
          provider: "Burny",
          plan: "Plan",
          state: "healthy",
          burnTokensPerMin: 100,
          windows: [
            {
              name: "weekly",
              unit: "tokens",
              limit: 100000,
              used: 90000,
              resetsAt: "2026-11-09T00:00:00Z",
              percent: 90,
            },
          ],
        },
      ],
    };
    const d = buildDigest(inputs({ quota: burning }));
    const outlook = section(d, "quota");
    expect(outlook.items).toHaveLength(1);
    expect(outlook.items[0].note).toContain("fills in ~");
    expect(outlook.items[0].note).not.toContain("before its");
  });

  test("done selects by completion time, not last update (#5279 Codex)", () => {
    const d = buildDigest(
      inputs({
        pool: [
          // Closed long ago, commented today: not done today.
          item({ number: 10, state: "closed", updatedAt: "2026-11-02T12:00:00Z", closedAt: "2026-10-20T08:00:00Z" }),
          // Merged this morning, untouched since: done today.
          item({
            number: 11,
            state: "merged",
            contentKind: "pr",
            updatedAt: "2026-10-20T08:00:00Z",
            mergedAt: "2026-11-02T07:00:00Z",
          }),
        ],
      }),
    );
    const done = section(d, "done");
    expect(done.items.map((i) => i.ref)).toEqual(["developing-today/code#11"]);
    expect(done.items[0].note).toContain("completed 2026-11-02T07:00:00Z");
  });

  test("quota outlook preserves provider error/stale states (#5279 Codex)", () => {
    const degraded: QuotaView = {
      ...quota,
      pools: [
        {
          id: "pool-down",
          provider: "Down",
          plan: "Plan",
          state: "error",
          freshness: { measuredAt: "2026-11-02T06:00:00Z", source: "gatherer", state: "error", error: "boom" },
          windows: [
            { name: "weekly", unit: "tokens", limit: 1000, used: 100, resetsAt: "2026-11-09T00:00:00Z", percent: 10 },
          ],
        },
        {
          id: "pool-stale",
          provider: "Stale",
          plan: "Plan",
          state: "stale",
          freshness: { measuredAt: "2026-11-01T06:00:00Z", source: "gatherer", state: "stale" },
          windows: [
            { name: "weekly", unit: "tokens", limit: 1000, used: 950, resetsAt: "2026-11-09T00:00:00Z", percent: 95 },
          ],
        },
      ],
    };
    const d = buildDigest(inputs({ quota: degraded }));
    const outlook = section(d, "quota");
    const byRef = new Map(outlook.items.map((i) => [i.ref, i.note]));
    expect(byRef.get("pool-down")).toContain("provider read error");
    expect(byRef.get("pool-down")).toContain("boom");
    expect(byRef.get("pool-stale")).toContain("provider read stale");
    expect(outlook.stale).toBe(true);
    expect(outlook.staleNote).toContain("Down");
  });

  test("chat text states freshness and links every section item", () => {
    const text = digestText(buildDigest(inputs()));
    expect(text).toContain("PM daily digest — 2026-11-02");
    expect(text).toContain("https://github.com/developing-today/code/issues/42");
    expect(text).toContain("Archived as: /digest?date=2026-11-02");
  });
});

describe("digest archive", () => {
  test("write → read round-trips; list sees the dated route", async () => {
    const dir = await mkdtemp(join(tmpdir(), "digest-archive-"));
    try {
      const d = buildDigest(inputs());
      await writeDigestArchive(dir, d);
      const back = await readDigestArchive(dir, "2026-11-02");
      expect(back?.generatedAt).toBe(d.generatedAt);
      expect(back?.sections).toHaveLength(5);
      expect(await readDigestArchive(dir, "2026-11-03")).toBeNull();
      expect(await listDigestArchives(dir)).toEqual([
        { kind: "daily", date: "2026-11-02", route: "/digest?date=2026-11-02" },
      ]);
      const weekly = buildDigest(inputs({ kind: "weekly" }));
      await writeDigestArchive(dir, weekly);
      const list = await listDigestArchives(dir);
      expect(list).toContainEqual({ kind: "weekly", date: "2026-11-02", route: "/digest?date=2026-11-02&kind=weekly" });
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
