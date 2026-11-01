// Usage & billing hub (design 2026-10-25): Part (b) extraction, PAYG
// lines, charges evidence, bubble rules, the snapshot store, pace
// projections, and the hub/detail renders. The sample view below
// carries the same dated-read phrasing quota-view.md uses, trimmed.
import { describe, expect, test } from "bun:test";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { buildAttentionFlags, freshness, projectPace, resetMarkers } from "../src/derive.ts";
import { countedEstimate, type QuotaView, type SnapshotRecord, type UsageView } from "../src/model.ts";
import { renderQuota, renderServiceDetail, fmtMetricValue, freshnessStamp } from "../src/render.ts";
import { extractCharges, extractPaygLines, extractSaasServices, extractSubscriptions } from "../src/saas-extract.ts";
import { isSnapshotRecord, SnapshotStore, snapshotFileFor } from "../src/snapshots.ts";
import { fixtureSource } from "../src/sources/fixtures.ts";

const VIEW = `
# Part (b) — Subscriptions & SaaS/infrastructure

## Summary (b) — subscriptions, infrastructure, account tooling

| Service | Plan / price | Allowance sketch | Usage / status | Next date recorded |
|---|---|---|---|---|
| Cloudflare platform | Workers Paid $5/mo + R2 Paid $0 base | 10M requests / 30M CPU-ms monthly | Historically far behind | Subscription date unknown |
| Turso | Starter Free $0 | 100 DBs, 5 GB, 500M reads/10M writes/3 GB syncs monthly | ~0% Oct 20 | 2026-10-31 cycle end |
| Modal | Starter $0/mo | $30/mo compute credits | unknown | Monthly; exact reset unknown |

## Service details (b)

### Cloudflare — Workers platform, R2 and AI gateway
- **Plan / price:** Tracker and Memory: Workers Paid $5/mo + R2 Paid $0 base, verified 2026-10-20. **Conflict/stale earlier evidence:** CF reports subscription endpoint 403 and presumed $0/free.
- **Monthly allowances:** Workers 10M requests/30M CPU-ms; logs 20M, 7-day retention; KV 10M reads, 1M writes/deletes/lists, 1 GB; D1 25B rows read/50M written, 5 GB; Queues 1M ops; Durable Objects 1M requests/400k GB-s/5 GB-mo SQL; Workflows 500k steps/1 GB-mo; Vectorize 50M queried/10M stored dimensions; Hyperdrive unlimited queries; Containers 25 GiB-hours memory/375 vCPU-min/200 GB-hours disk (CF).
- **R2 / daily allowances / billing:** R2 Standard 10 GB-mo, 1M Class A and 10M Class B operations monthly; egress $0. Workers AI 10k Neurons/day; paid excess $0.011/1k Neurons.
- **Usage / pace:** Tracker: small, 3 Workers, 2 R2 buckets ~2 GB, 2 D1, 2 KV, 2 Queues; far behind, mostly unused.
- **PAYG estimate (excess only, excluding the $5/mo base which is counted as subscription): low $0 / high $5.20 per month; counted as used = 1.2 × high = $6.24 per month.** Basis: every meter sits inside its included allotment.
- **Last checked:** Plan/footprint 2026-10-20, token 2026-10-23; gateway observation 2026-10-25.

### Turso
- **Plan / price / allowance:** Starter Free $0: 100 DBs, 5 GB storage, 500M reads/month, 10M writes/month, 3 GB syncs/month, 1-day PITR.
- **Usage / dates:** ~0%, far behind: 1 DB \`great-star-lord\`, 0 B, 0 usage on 2026-10-20; cycle Sep 30–Oct 31 2026. Mirror observation week ends 2026-10-29, not a billing reset.
- **PAYG estimate:** not applicable — Starter Free is a $0 hard-capped plan with overages disabled; there is no metered bill to estimate.
- **Last checked:** Plan/dashboard 2026-10-20; overage setting 2026-10-21; mirror capability 2026-10-24.

### Modal
- **Plan / price / allowance:** Starter $0/mo, Drew confirmed 2026-10-22; $30 monthly compute-credit grant, per-second billing past credits.
- **Usage / dates:** Percentage/pace unknown; monthly grant, exact reset/credit expiry date unknown.
- **PAYG estimate: not estimable from local records** — no run hours or credit balance are recorded.
- **Last checked:** 2026-10-22 (Tracker).

### Meta Model API
- **Usage / dates:** weekly quota **46% at 03:20 CDT**; morning sessions spilled to PAYG — **$23.91 charged** ($20.00 at 05:15 CDT + $3.91 at 07:47 CDT). **Console read (dev.meta.ai, 2026-10-25 09:38 CDT):** weekly limit **85% used at 09:38 CDT**. PAYG Oct 19–25: $23.92 accrued / $23.91 collected, nothing outstanding.
`;

const REGISTRY = `
## Billing incidents + Drew's response (2026-10-25, main chat)

- [observed] Meta API plan: usage past the wall metered as PAYG — **$20.00 charged 05:15 CDT + $3.91 charged 07:47 CDT** (Visa •7535 card alerts).
- [observed] Cloudflare gateway DeepSeek V4 Pro accrual Oct 24–25: $29.60 per budget alert vs $45.10 per gateway cost attribution; final figure lands on the Nov 2 invoice.
`;

describe("saas extraction (Part b)", () => {
  const services = extractSaasServices(VIEW);
  const byId = new Map(services.map((s) => [s.id, s]));

  test("cloudflare: one card, highlight limits + long tail, estimate, conflict", () => {
    const cf = byId.get("cloudflare")!;
    expect(cf.plan).toContain("Workers Paid $5/mo");
    const m = new Map(cf.metrics.map((x) => [x.id, x]));
    expect(m.get("workers-requests")?.limit).toBe(10_000_000);
    expect(m.get("workers-cpu-ms")?.limit).toBe(30_000_000);
    expect(m.get("d1-rows-read")?.limit).toBe(25_000_000_000);
    expect(m.get("workers-ai-neurons")?.limit).toBe(10_000);
    expect(m.get("r2-storage")?.used).toBe(2);
    expect(m.get("r2-storage")?.percent).toBe(20);
    expect(m.get("workflows-steps")?.highlight).toBe(false); // long tail stays off the front page
    expect(cf.paygEstimate).toEqual({ low: 0, high: 5.2, period: "per month", note: expect.any(String) });
    expect(countedEstimate(cf.paygEstimate!)).toBe(6.24);
    expect(cf.conflicts.length).toBeGreaterThan(0);
    expect(cf.measuredAt).toBe("2026-10-25");
  });

  test("turso: measuredAt is the last-checked date, not a future operational date", () => {
    const t = byId.get("turso")!;
    expect(t.measuredAt).toBe("2026-10-24"); // not 2026-10-29 (mirror review week)
    const m = new Map(t.metrics.map((x) => [x.id, x]));
    expect(m.get("databases")?.used).toBe(1);
    expect(m.get("databases")?.limit).toBe(100);
    expect(m.get("rows-read")?.limit).toBe(500_000_000);
    expect(t.paygEstimate).toBeNull();
    expect(t.paygNote).toContain("not applicable");
    expect(t.nextDate).toBe("2026-10-31 cycle end");
  });

  test("modal: not-estimable stays a note, never a number", () => {
    const modal = byId.get("modal")!;
    expect(modal.paygEstimate).toBeNull();
    expect(modal.paygNote).toBe("not estimable from local records");
    expect(modal.metrics[0]?.limit).toBe(30);
  });

  test("missing sections render as identity shells, never vanish", () => {
    const ids = services.map((s) => s.id);
    for (const id of ["supabase", "github", "vercel", "oci", "bitwarden"]) expect(ids).toContain(id);
    expect(byId.get("vercel")?.usageNote).toContain("baseline");
  });

  test("fail-soft on an empty document", () => {
    const shells = extractSaasServices("nothing here");
    expect(shells.length).toBe(11);
    expect(shells.every((s) => s.measuredAt === null)).toBe(true);
  });

  test("candidates: B2, Fly.io, Baseten are explicit candidate shells, never held (#1058)", () => {
    for (const id of ["backblaze-b2", "fly-io", "baseten"]) {
      const s = byId.get(id)!;
      expect(s).toBeDefined();
      expect(s.status).toBe("candidate");
      expect(s.metrics).toEqual([]);
      expect(s.measuredAt).toBeNull();
      expect(s.plan).toContain("Candidate — not held");
    }
  });

  test("sources provenance: tracker row, baseline, quota-view section (#1058)", () => {
    const t = byId.get("turso")!;
    expect(t.sources?.quotaViewSection).toBe("### Turso");
    expect(t.sources?.trackerRow).toBe("Turso");
    expect(t.sources?.baselineDoc).toContain("evidence-and-interpretation-baseline-2026-10-20.md");
    expect(byId.get("fly-io")!.sources?.trackerRow).toBeNull();
  });

  test("modal: monthly console read carries a dated used-credits value", () => {
    const view = VIEW.replace(
      "- **Usage / dates:** Percentage/pace unknown; monthly grant, exact reset/credit expiry date unknown.",
      "- **Usage / dates:** $12.50 compute credits used on 2026-10-24; monthly grant, exact reset unknown.",
    ).replace("- **Last checked:** 2026-10-22 (Tracker).", "- **Last checked:** console read 2026-10-24.");
    const modal = new Map(extractSaasServices(view).map((s) => [s.id, s])).get("modal")!;
    const credits = modal.metrics.find((m) => m.id === "compute-credits")!;
    expect(credits.used).toBe(12.5);
    expect(credits.limit).toBe(30);
    expect(credits.percent).toBeCloseTo(41.7, 1);
    expect(modal.measuredAt).toBe("2026-10-24");
  });

  test("modal: percentage read is not mistaken for dollars", () => {
    const view = VIEW.replace(
      "- **Usage / dates:** Percentage/pace unknown; monthly grant, exact reset/credit expiry date unknown.",
      "- **Usage / dates:** used 50% of monthly compute credits on 2026-10-24; monthly grant, exact reset unknown.",
    );
    const modal = new Map(extractSaasServices(view).map((s) => [s.id, s])).get("modal")!;
    const credits = modal.metrics.find((m) => m.id === "compute-credits")!;
    expect(credits.used).toBeNull();
  });

  test("modal: no used figure stays an allowance-only shell", () => {
    const modal = byId.get("modal")!;
    expect(modal.metrics.find((m) => m.id === "compute-credits")?.used).toBeNull();
  });

  test("bitwarden: bws count-only read yields dated secrets/projects metrics", () => {
    const view = `${VIEW}
### Bitwarden
- **Plan / price / allowance:** Secrets Manager; plan/price unknown.
- **Usage / dates:** 121 secrets, 1 project (bws count-only read 2026-10-26).
- **PAYG estimate:** not applicable — flat account-tooling plan.
- **Last checked:** bws count-only read 2026-10-26.
`;
    const bw = new Map(extractSaasServices(view).map((s) => [s.id, s])).get("bitwarden")!;
    const m = new Map(bw.metrics.map((x) => [x.id, x]));
    expect(m.get("secrets-count")?.used).toBe(121);
    expect(m.get("projects-count")?.used).toBe(1);
    expect(bw.measuredAt).toBe("2026-10-26");
  });

  test("bitwarden: no dated count stays an empty shell, names/values never parsed", () => {
    const bw = byId.get("bitwarden")!;
    expect(bw.metrics).toEqual([]);
  });
});

describe("payg lines + charges (§1.2, §2.4)", () => {
  test("meta console figures: accrued vs collected, unbilled is the remainder", () => {
    const lines = extractPaygLines(VIEW, REGISTRY);
    const meta = lines.find((l) => l.id === "meta-payg")!;
    expect(meta.accruedUsd).toBe(23.92);
    expect(meta.collectedUsd).toBe(23.91);
    expect(meta.unbilledUsd).toBe(0.01);
  });

  test("gateway disagreement is shown as a low/high estimate pair", () => {
    const gw = extractPaygLines(VIEW, REGISTRY).find((l) => l.id === "cloudflare-gateway")!;
    expect(gw.estimate).toMatchObject({ low: 29.6, high: 45.1 });
    expect(gw.unbilledUsd).toBe(54.12); // counted = 1.2 × high
  });

  test("charges carry their evidence source and payment method", () => {
    const charges = extractCharges(VIEW, REGISTRY);
    const meta = charges.filter((c) => c.label === "Meta PAYG spill");
    expect(meta.map((c) => c.amountUsd).sort((a, b) => a - b)).toEqual([3.91, 20]);
    expect(meta[0].method).toBe("Visa •7535");
    expect(meta[0].source).toContain("card alert");
  });

  test("single dated Meta charge incidents land as their own rows, deduped across both records", () => {
    const registry = `- [observed — Gmail card alert, 2026-10-26 10:08 CDT] Meta PAYG (METAPAY) —\n  **$20.00 charged 10:08 CDT** (Citi Costco card ending 9545; merchant "Metapay*uc5dpa22y4").`;
    const view = `### Meta Model API\n- **Usage / dates:** **Newer dated charge (2026-10-26): $20.00 charged 10:08 CDT — Meta PAYG (METAPAY), merchant "Metapay*uc5dpa22y4", on Citi Costco card ending 9545 (Gmail card alert).**`;
    const meta = extractCharges(view, registry).filter((c) => c.label === "Meta PAYG spill");
    expect(meta).toEqual([
      {
        at: "2026-10-26",
        label: "Meta PAYG spill",
        amountUsd: 20,
        method: "Citi Costco •9545",
        source: "Gmail card alert (registry billing incidents)",
      },
    ]);
  });

  test("subscriptions: a non-calendar renews date is null, a leap day is kept (#2206)", () => {
    const wispr = (date: string) =>
      extractSubscriptions(`### Wispr Flow
- **Plan:** Flow Pro annual $151.91/yr charged; renews ${date}.`).find((s) => s.name === "Wispr Flow");
    expect(wispr("2026-02-30")?.renewsAt).toBeNull();
    expect(wispr("2026-04-31")?.renewsAt).toBeNull();
    expect(wispr("2026-13-01")?.renewsAt).toBeNull();
    expect(wispr("2024-02-29")?.renewsAt).toBe("2024-02-29");
    expect(wispr("2027-10-09")?.renewsAt).toBe("2027-10-09");
  });

  test("subscriptions strip is renewal-only rows from the records", () => {
    const subs = extractSubscriptions(
      `${VIEW}\n### Other / flat (b)\n- **Amazon RxPass:** Subscription $5/mo; flat pharmacy.\n### Wispr Flow\n- **Plan:** Flow Pro annual $151.91/yr charged; renews 2027-10-09.`,
    );
    expect(subs).toEqual([
      { name: "Amazon RxPass", plan: "$5/mo flat pharmacy", renewsAt: null },
      { name: "Wispr Flow", plan: "Pro $151.91/yr", renewsAt: "2027-10-09" },
    ]);
  });
});

describe("bubble rules (§1.3)", () => {
  const quota: QuotaView = {
    pools: [
      {
        id: "p1",
        provider: "P",
        plan: "Plan",
        state: "healthy",
        windows: [
          { name: "at-80", unit: "other", limit: 100, used: 80, resetsAt: null },
          { name: "over-80", unit: "other", limit: 100, used: 81, resetsAt: null },
        ],
      },
    ],
    burn: [],
    choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] },
  };
  test("strictly over 80% bubbles; exactly 80% does not", () => {
    const flags = buildAttentionFlags(quota, { services: [], payg: [] });
    expect(flags.map((f) => f.label)).toEqual(["P · Plan — over-80"]);
  });
  test("projected unbilled strictly over $5 bubbles", () => {
    const flags = buildAttentionFlags(quota, {
      services: [],
      payg: [
        { id: "a", label: "A", accruedUsd: null, unbilledUsd: 5, note: "", measuredAt: null, sourceClass: "estimate" },
        {
          id: "b",
          label: "B",
          accruedUsd: null,
          unbilledUsd: 5.01,
          note: "",
          measuredAt: null,
          sourceClass: "estimate",
        },
      ],
    });
    expect(flags.filter((f) => f.kind === "payg-unbilled").map((f) => f.label)).toEqual(["B"]);
  });
});

describe("snapshot store (§4)", () => {
  const rec = (ts: string, value: number): SnapshotRecord => ({
    ts,
    serviceId: "turso",
    metricId: "usage",
    value,
    unit: "pct",
    limit: 100,
    source: "manual",
    observedBy: "test",
  });
  test("monthly file naming follows the record's own month", () => {
    expect(snapshotFileFor("2026-10-25T09:38:00-05:00")).toBe("usage-2026-10.jsonl");
    expect(snapshotFileFor("2026-09-30T12:00:00Z")).toBe("usage-2026-09.jsonl");
  });
  test("validator refuses negative numeric pct values, other units unchanged (#2346)", () => {
    expect(isSnapshotRecord(rec("2026-10-25T12:00:00Z", -1))).toBe(false);
    expect(isSnapshotRecord(rec("2026-10-25T12:00:00Z", -1e9))).toBe(false);
    expect(isSnapshotRecord(rec("2026-10-25T12:00:00Z", 0))).toBe(true);
    expect(isSnapshotRecord({ ...rec("2026-10-25T12:00:00Z", -5), unit: "usd" })).toBe(true);
  });
  test("stored poisoned pct records read back through readAll never make projectPace throw (#2346)", async () => {
    const dir = await mkdtemp(join(tmpdir(), "hub-snaps-"));
    try {
      // Written directly to the month file, bypassing append: records
      // stored before the validator guard existed.
      const poison = [rec("2026-10-22T00:00:00Z", -1e9), rec("2026-10-24T00:00:00Z", -999999999)];
      await writeFile(join(dir, "usage-2026-10.jsonl"), poison.map((r) => JSON.stringify(r)).join("\n") + "\n");
      const all = await new SnapshotStore(dir).readAll();
      expect(() => projectPace(all, "usage")).not.toThrow();
      // The primary guard itself, for a poisoned set that reaches
      // projectPace in memory: no projection, no throw.
      const pace = projectPace(poison, "usage")!;
      expect(pace.projectedExhaustionAt).toBeNull();
      expect(pace.slopePerDay).toBe(0.5);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
  test("append is idempotent and reads survive a fresh store instance", async () => {
    const dir = await mkdtemp(join(tmpdir(), "hub-snaps-"));
    try {
      const store = new SnapshotStore(dir);
      expect(await store.append([rec("2026-10-20T12:00:00Z", 0), rec("2026-10-25T12:00:00Z", 40)])).toBe(2);
      expect(await store.append([rec("2026-10-20T12:00:00Z", 0)])).toBe(0); // dedupe
      const byService = await new SnapshotStore(dir).byService();
      expect(byService["turso"]?.map((r) => r.value)).toEqual([0, 40]);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
  test("estimate pairs round-trip as pairs (the 1.2x rule is render-side)", async () => {
    const dir = await mkdtemp(join(tmpdir(), "hub-snaps-"));
    try {
      const store = new SnapshotStore(dir);
      await store.append([
        {
          ts: "2026-10-25T12:00:00Z",
          serviceId: "cloudflare",
          metricId: "est",
          value: { low: 29.6, high: 45.1 },
          unit: "usd-est",
          source: "estimate",
          observedBy: "test",
        },
      ]);
      const all = await store.readAll();
      expect(all[0].value).toEqual({ low: 29.6, high: 45.1 });
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});

describe("pace + freshness (§4, §1.1)", () => {
  const rec = (ts: string, value: number, resetsAt?: string): SnapshotRecord => ({
    ts,
    serviceId: "x",
    metricId: "weekly",
    value,
    unit: "pct",
    limit: 100,
    ...(resetsAt ? { resetsAt } : {}),
    source: "console",
    observedBy: "test",
  });
  test("pace projects exhaustion from the current window only", () => {
    const recs = [
      rec("2026-10-20T00:00:00Z", 90, "2026-10-21T00:00:00Z"),
      rec("2026-10-22T00:00:00Z", 10, "2026-10-29T00:00:00Z"),
      rec("2026-10-24T00:00:00Z", 30, "2026-10-29T00:00:00Z"),
    ];
    const pace = projectPace(recs, "weekly")!;
    expect(pace.points).toBe(2); // the pre-reset point is excluded
    expect(pace.slopePerDay).toBe(10);
    expect(pace.projectedExhaustionAt?.slice(0, 10)).toBe("2026-10-31");
    expect(pace.projectedPercentAtReset).toBe(80);
    expect(resetMarkers(recs, "weekly")).toHaveLength(1);
  });
  test("pace: out-of-domain pct yields no projection and never throws (#2346)", () => {
    // -1e9 -> -999999999 over 2 days: slope 0.5/day passes the slope
    // gate; the unguarded extrapolation threw RangeError.
    const poison = projectPace([rec("2026-10-22T00:00:00Z", -1e9), rec("2026-10-24T00:00:00Z", -999999999)], "weekly")!;
    expect(poison.projectedExhaustionAt).toBeNull();
    expect(poison.slopePerDay).toBe(0.5);
    // -5000 -> -4999.88 over 2 days: slope 0.06/day; previously
    // projected exhaustion in the year 2259.
    const far = projectPace([rec("2026-10-22T00:00:00Z", -5000), rec("2026-10-24T00:00:00Z", -4999.88)], "weekly")!;
    expect(far.projectedExhaustionAt).toBeNull();
    // Happy-path control: 30 -> 40 over 2 days projects as before.
    const ok = projectPace([rec("2026-10-22T00:00:00Z", 30), rec("2026-10-24T00:00:00Z", 40)], "weekly")!;
    expect(ok.projectedExhaustionAt?.slice(0, 10)).toBe("2026-11-05");
  });
  test("no projection from a single point or a flat series", () => {
    expect(projectPace([rec("2026-10-24T00:00:00Z", 30)], "weekly")).toBeNull();
    expect(projectPace([rec("2026-10-22T00:00:00Z", 30), rec("2026-10-24T00:00:00Z", 30)], "weekly")).toBeNull();
  });
  test("freshness: older than cadence is stale; same-day is not", () => {
    const now = new Date("2026-10-26T12:00:00Z");
    expect(freshness("2026-10-25", 7, now)).toEqual({ stale: false, ageDays: 1 });
    expect(freshness("2026-10-01", 7, now).stale).toBe(true);
    expect(freshness(null, 7, now)).toEqual({ stale: true, ageDays: null });
  });
  test("freshness: date-only stamps age by the Chicago calendar day (#1629)", () => {
    // 19:30 CDT on 2026-10-25 is already 2026-10-26 in UTC. A read
    // dated today (Chicago) is 0 days old for the whole Chicago day;
    // the UTC-day age called it 1d old -- and stale at cadence 0 --
    // from 19:00 Chicago onward.
    const cdtEvening = new Date("2026-10-26T00:30:00Z");
    expect(freshness("2026-10-25", 7, cdtEvening)).toEqual({ stale: false, ageDays: 0 });
    expect(freshness("2026-10-25", 0, cdtEvening)).toEqual({ stale: false, ageDays: 0 });
    expect(freshness("2026-10-24", 7, cdtEvening)).toEqual({ stale: false, ageDays: 1 });
    expect(freshness("2026-10-24", 0, cdtEvening)).toEqual({ stale: true, ageDays: 1 });
    // Still today at 23:59 Chicago.
    expect(freshness("2026-10-25", 7, new Date("2026-10-26T04:59:00Z"))).toEqual({
      stale: false,
      ageDays: 0,
    });
    // The CST evening of the same shape: 19:00 CST on 2026-01-14 is
    // 2026-01-15 in UTC.
    const cstEvening = new Date("2026-01-15T01:00:00Z");
    expect(freshness("2026-01-14", 7, cstEvening)).toEqual({ stale: false, ageDays: 0 });
    expect(freshness("2026-01-13", 7, cstEvening)).toEqual({ stale: false, ageDays: 1 });
    // Tomorrow's Chicago date is still within the future tolerance.
    expect(freshness("2026-10-26", 7, cdtEvening)).toEqual({ stale: false, ageDays: 0 });
  });
  test("freshness: a far-future stamp is stale with unknown age (#1568)", () => {
    const now = new Date("2026-10-26T12:00:00Z");
    // Date-only form: +30d and +365d are beyond the 1-day tolerance.
    expect(freshness("2026-11-25", 7, now)).toEqual({ stale: true, ageDays: null });
    expect(freshness("2027-10-26", 7, now)).toEqual({ stale: true, ageDays: null });
    // Datetime form: +30d, +365d, and even +2d are beyond tolerance.
    expect(freshness("2026-11-25T12:00:00Z", 7, now)).toEqual({ stale: true, ageDays: null });
    expect(freshness("2027-10-26T12:00:00Z", 7, now)).toEqual({ stale: true, ageDays: null });
    expect(freshness("2026-10-28T12:00:00Z", 7, now)).toEqual({ stale: true, ageDays: null });
    // Garbage is the same honest unknown-age shape.
    expect(freshness("not-a-date-at-all", 7, now)).toEqual({ stale: true, ageDays: null });
  });
  test("freshness: a near-future stamp within tolerance is today, not stale (#1568)", () => {
    const now = new Date("2026-10-26T12:00:00Z");
    // Date-only granularity: tomorrow's date is exactly at the tolerance.
    expect(freshness("2026-10-27", 7, now)).toEqual({ stale: false, ageDays: 0 });
    // Datetime: +12h of timezone/clock noise clamps to age 0.
    expect(freshness("2026-10-27T00:00:00Z", 7, now)).toEqual({ stale: false, ageDays: 0 });
    // Past controls unchanged.
    expect(freshness("2026-10-25", 7, now)).toEqual({ stale: false, ageDays: 1 });
    expect(freshness("2026-10-18", 7, now)).toEqual({ stale: true, ageDays: 8 });
  });
  test("freshnessStamp: a far-future stamp renders the stale badge, never 'today' (#1568)", () => {
    const future = new Date(Date.now() + 30 * 86_400_000).toISOString().slice(0, 10);
    const html = freshnessStamp(future, "console", 7);
    expect(html).toContain("stale — as of");
    expect(html).not.toContain("· today");
  });
});

describe("hub renders", () => {
  test("fixture source: hub page carries both families + the strip", async () => {
    const src = fixtureSource();
    const html = renderQuota(await src.quota(), src.provenance(), await src.usage());
    expect(html).toContain("Usage &amp; billing");
    expect(html).toContain("Needs attention");
    expect(html).toContain("PAYG dollars");
    expect(html).toContain("Charges this period");
    expect(html).toContain("/quota/service/turso");
    expect(html).toContain("measured");
  });
  test("PAYG card shows the DeepSeek ¥ balance with its measured-at (#1052)", async () => {
    const src = fixtureSource();
    const usage = await src.usage();
    const rec: SnapshotRecord = {
      ts: "2026-10-25T12:00:00Z",
      serviceId: "deepseek:api",
      metricId: "credits_balance",
      value: 44.63,
      unit: "cny",
      source: "api",
      observedBy: "backfill:quota-view.md",
    };
    usage.history = { ...usage.history, "deepseek:api": [rec] };
    const html = renderQuota(await src.quota(), src.provenance(), usage);
    expect(html).toContain("DeepSeek PAYG reserve balance");
    expect(html).toContain("¥44.63");
    expect(html).toContain("measured 2026-10-25");
  });
  test("no cny record → no balance line; fmtMetricValue renders cny in yuan", async () => {
    const src = fixtureSource();
    const html = renderQuota(await src.quota(), src.provenance(), await src.usage());
    expect(html).not.toContain("DeepSeek PAYG reserve balance");
    expect(fmtMetricValue(44.63, "cny")).toBe("¥44.63");
    expect(fmtMetricValue(null, "cny")).toBe("not measured");
  });
  test("legacy two-arg renderQuota still renders the pools page", async () => {
    const src = fixtureSource();
    const html = renderQuota(await src.quota(), src.provenance());
    expect(html).toContain("Quota + throughput");
  });
  test("hub sources section + candidate cards visible; detail sources block (#1058)", async () => {
    const src = fixtureSource();
    const usage: UsageView = {
      services: extractSaasServices(VIEW),
      payg: extractPaygLines(VIEW, REGISTRY),
      charges: [],
      attention: [],
      domains: null,
      subscriptions: [],
      history: {},
    };
    const quota: QuotaView = { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } };
    const html = renderQuota(quota, src.provenance(), usage);
    expect(html).toContain("free-services-to-check.md");
    expect(html).toContain("evidence-and-interpretation-baseline-2026-10-20.md");
    expect(html).toContain("Backblaze B2");
    expect(html).toContain("Fly.io");
    expect(html).toContain("Baseten");
    expect(html).toContain("candidate");
    const turso = usage.services.find((s) => s.id === "turso")!;
    const detail = renderServiceDetail(turso, [], src.provenance());
    expect(detail).toContain("Sources");
    expect(detail).toContain("Summary (b) — Turso");
    expect(detail).toContain("### Turso");
    expect(detail).toContain("evidence-and-interpretation-baseline-2026-10-20.md");
    const fly = usage.services.find((s) => s.id === "fly-io")!;
    const flyDetail = renderServiceDetail(fly, [], src.provenance());
    expect(flyDetail).toContain("candidate / not held");
    // An active service with no Summary (b) row is not thereby a candidate:
    // ownership is never inferred from a missing tracker row (Codex, PR #3677).
    const rowless = {
      ...turso,
      sources: turso.sources ? { ...turso.sources, trackerRow: null } : turso.sources,
    };
    expect(rowless.status).toBe("active");
    const rowlessDetail = renderServiceDetail(rowless, [], src.provenance());
    expect(rowlessDetail).toContain("none — no Summary (b) row");
    expect(rowlessDetail).not.toContain("candidate / not held");
  });
  test("service detail: full metrics, chart, reads, conflicts", async () => {
    const src = fixtureSource();
    const usage = await src.usage();
    const turso = usage.services.find((s) => s.id === "turso")!;
    const html = renderServiceDetail(turso, usage.history["turso"] ?? [], src.provenance());
    expect(html).toContain("All metrics");
    expect(html).toContain("Snapshot history");
    expect(html).toContain("<svg");
    expect(html).toContain("Pace projections");
    const cf = usage.services.find((s) => s.id === "cloudflare")!;
    const cfHtml = renderServiceDetail(cf, [], src.provenance());
    expect(cfHtml).toContain("Conflicts (shown, not resolved)");
    expect(cfHtml).toContain("counted as used");
  });
  test("service detail: stored snapshot freshness follows the service cadence", () => {
    const turso = extractSaasServices(VIEW).find((s) => s.id === "turso")!;
    const now = new Date("2026-11-01T12:00:00Z");
    const record = (ts: string): SnapshotRecord => ({
      ts,
      serviceId: "turso",
      metricId: "databases",
      value: 1,
      unit: "count",
      source: "manual",
      observedBy: "test",
    });
    const current = renderServiceDetail(
      turso,
      [record("2026-10-30T12:00:00Z")],
      fixtureSource().provenance(),
      undefined,
      now,
    );
    expect(current).toContain("Latest stored snapshot:");
    expect(current).toContain("2026-10-30");
    expect(current).toContain("7-day expected cadence");
    expect(current).toContain("within cadence");

    const stale = renderServiceDetail(
      turso,
      [record("2026-10-01T12:00:00Z")],
      fixtureSource().provenance(),
      undefined,
      now,
    );
    expect(stale).toContain("stale");

    const empty = renderServiceDetail(turso, [], fixtureSource().provenance(), undefined, now);
    expect(empty).toContain("No snapshot writer has landed a record for this service yet");
    expect(empty).toContain("one-shot recovery");
  });
});
