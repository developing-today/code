// Phase 3 of #2576 (#2579): extractor and mapper boundaries on the
// shared validate.ts primitives. Each block reproduces its issue's
// malformed-input shape and asserts the sanctioned outcome for that
// boundary — retain (error entry), drop-and-count, or refuse — never a
// throw, a fabricated row, or live out-of-range data.
import { describe, expect, test } from "bun:test";
import { applySupabaseExport, extractSaasServices, type SupabaseExportDoc } from "../src/saas-extract.ts";
import { entryFromFileExport, entryFromResult, entryFromTurso } from "../src/quota-gather-map.ts";
import { extractQuotaPools } from "../src/quota-extract.ts";
import { aggregateBillingUsage } from "../src/sources/github-billing.ts";
import { droppedJulesItemCount, mapJulesItems } from "../src/sources/jules.ts";

const NOW = Date.parse("2026-10-29T15:00:00Z");

function metaDoc(overrides: Record<string, unknown> = {}) {
  return {
    service: "meta",
    status: "ok",
    measuredAt: "2026-10-29T14:55:00Z",
    windows: [{ name: "5h", usedPct: 42, resetAt: "2026-10-29T20:00:00Z" }],
    ...overrides,
  } as Parameters<typeof entryFromFileExport>[0];
}

describe("entryFromFileExport (#2551 #2571)", () => {
  test("control document maps its window", () => {
    const entry = entryFromFileExport(metaDoc(), { fileMtimeIso: null, stale: false });
    expect(entry.state).toBe("ok");
    expect(entry.windows.map((w) => [w.name, w.value])).toEqual([["5h", 42]]);
  });

  test("a null window element returns the unparseable outcome, never throws", () => {
    const entry = entryFromFileExport(metaDoc({ windows: [null] }), { fileMtimeIso: null, stale: false });
    expect(entry.state).toBe("error");
    expect(entry.windows).toEqual([]);
    expect(entry.error).toContain("unparseable");
  });

  test("a non-string window name returns the unparseable outcome", () => {
    const entry = entryFromFileExport(metaDoc({ windows: [{ name: 42, usedPct: 10 }] }), {
      fileMtimeIso: null,
      stale: false,
    });
    expect(entry.state).toBe("error");
    expect(entry.windows).toEqual([]);
  });

  test("an out-of-range percent is dropped, its valid sibling survives", () => {
    const entry = entryFromFileExport(
      metaDoc({
        windows: [
          { name: "5h", usedPct: -20 },
          { name: "weekly", usedPct: 1e9 },
          { name: "monthly", usedPct: 69 },
        ],
      }),
      { fileMtimeIso: null, stale: false },
    );
    expect(entry.state).toBe("ok");
    expect(entry.windows.map((w) => [w.name, w.value])).toEqual([["monthly", 69]]);
  });

  test("an impossible measured date is not rolled into the next month", () => {
    const entry = entryFromFileExport(metaDoc({ measuredAt: "2026-02-30T00:00:00Z" }), {
      fileMtimeIso: null,
      stale: false,
    });
    expect(entry.state).toBe("error");
    expect(entry.windows).toEqual([]);
  });
});

describe("entryFromResult (#2571)", () => {
  const result = (windows: Record<string, unknown>) =>
    entryFromResult(
      {
        providerId: "claude",
        ok: true,
        configured: true,
        usage: { windows },
      },
      "2026-10-29T15:00:00Z",
    );

  test("out-of-range percents are dropped; a valid sibling maps", () => {
    const entry = result({
      fiveHour: { usedPercent: -20 },
      weekly: { usedPercent: 1e9 },
      monthly: { usedPercent: 55, resetAt: "2026-11-01T00:00:00Z" },
    });
    expect(entry.windows.map((w) => [w.name, w.value])).toEqual([["monthly", 55]]);
  });

  test("a missing percent stays a null-valued window, not a fabricated zero", () => {
    const entry = result({ weekly: {} });
    expect(entry.windows.map((w) => [w.name, w.value])).toEqual([["weekly", null]]);
  });
});

describe("entryFromTurso (#2553)", () => {
  test("negative counts are omitted, never published as live ok data", () => {
    const entry = entryFromTurso(
      { organization: { usage: { databases: -4, rows_read: 120, storage_bytes: 1_000_000_000 } } } as never,
      null,
      { measuredAt: "2026-10-29T15:00:00Z" },
    );
    expect(entry.state).toBe("ok");
    const names = entry.windows.map((w) => w.name);
    expect(names).toContain("rows-read");
    expect(names).not.toContain("databases");
    expect(entry.windows.every((w) => (w.value ?? 0) >= 0)).toBe(true);
  });
});

const SUPABASE_VIEW = `
## Summary (b) — subscriptions, infrastructure, account tooling

| Service | Plan / price | Allowance sketch | Usage / status | Next date recorded |
|---|---|---|---|---|
| Supabase | Free plan (muse-dev) | DB + egress + storage | manual dashboard read needed | unknown |

## Service details (b)

### Supabase
- **Plan / price:** Free plan (muse-dev); numerical quotas unknown in local records.
- **Usage / dates:** Not measured — manual dashboard read needed.
- **Last checked:** 2026-10-28.
`;

function supabaseDoc(overrides: Partial<SupabaseExportDoc> = {}): SupabaseExportDoc {
  return {
    service: "supabase",
    source: "file-export/supabase",
    status: "partial",
    measuredAt: "2026-10-29T14:55:00Z",
    project: "muse-dev",
    metrics: [{ id: "egress", label: "Egress", used: 1.25, limit: 5, unit: "gb" }],
    billing: { measured: true, billedUsd: 0 },
    ...overrides,
  };
}

describe("applySupabaseExport (#2508)", () => {
  test("a non-string measuredAt cannot pass the freshness gate", () => {
    const services = extractSaasServices(SUPABASE_VIEW);
    const out = applySupabaseExport(services, supabaseDoc({ measuredAt: [2026, 10, 29] as never }), NOW);
    expect(out).toEqual(services);
  });

  test("an impossible measuredAt is rejected, not rolled over", () => {
    const services = extractSaasServices(SUPABASE_VIEW);
    const out = applySupabaseExport(services, supabaseDoc({ measuredAt: "2026-02-30T00:00:00Z" }), NOW);
    expect(out).toEqual(services);
  });

  test("negative used/limit are dropped; valid siblings still apply", () => {
    const services = extractSaasServices(SUPABASE_VIEW);
    const out = applySupabaseExport(
      services,
      supabaseDoc({
        metrics: [
          { id: "egress", label: "Egress", used: 1.25, limit: 5, unit: "gb" },
          { id: "db-size", label: "DB", used: -1, limit: 0.5, unit: "gb" },
          { id: "storage", label: "Storage", used: 0.5, limit: -2, unit: "gb" },
        ],
      }),
      NOW,
    );
    const supabase = out.find((s) => s.id === "supabase")!;
    const egress = supabase.metrics.find((m) => m.id === "egress")!;
    expect(egress.used).toBe(1.25);
    const db = supabase.metrics.find((m) => m.id === "db-size")!;
    expect(db.used).not.toBe(-1);
  });

  test("negative billedUsd is refused", () => {
    const services = extractSaasServices(SUPABASE_VIEW);
    const out = applySupabaseExport(services, supabaseDoc({ billing: { measured: true, billedUsd: -5 } }), NOW);
    expect(out.find((s) => s.id === "supabase")!.billedUsd).toBeNull();
  });
});

const CLINE_VIEW = (measured: string, offset: string) => `
### ClinePass

Measured ${measured} ~04:48 CDT

5-hour **10%** used; weekly **26%** used (resets in ${offset}d 23h); monthly **69%** used (resets in ${offset}d 15h)
`;

describe("extractClinePass date anchors (#2383)", () => {
  const pool = (view: string) => extractQuotaPools(view, new Date("2026-10-29T12:00:00Z")).find((p) => p.id === "cline-pass:cline-pass")!;

  test("control read anchors both resets to a real measured date", () => {
    const p = pool(CLINE_VIEW("2026-10-25", "5"));
    expect(p.windows.map((w) => w.name)).toEqual(["5-hour", "weekly", "monthly"]);
    expect(p.windows.find((w) => w.name === "weekly")!.resetsAt).toBe("2026-10-31T08:48:00.000Z");
  });

  test("an impossible measured date makes only this pool's read unparseable", () => {
    const p = pool(CLINE_VIEW("2026-02-30", "5"));
    expect(p.windows).toEqual([]);
  });

  test("an oversized reset offset is dropped, the percentages survive", () => {
    const p = pool(CLINE_VIEW("2026-10-25", "200000"));
    expect(p.windows.map((w) => w.name)).toEqual(["5-hour", "weekly", "monthly"]);
    expect(p.windows.find((w) => w.name === "weekly")!.resetsAt).toBeNull();
  });
});

describe("sources/jules wire containers (#2531)", () => {
  test("a null item is dropped and counted; valid siblings map", () => {
    const state = { items: { a: null, b: { state: "IN_PROGRESS", title: "ok" } } };
    expect(droppedJulesItemCount(state)).toBe(1);
    expect(mapJulesItems(state).map((e) => e.sessionId)).toEqual(["b"]);
  });

  test("a string items container fabricates no phantom sessions", () => {
    expect(mapJulesItems({ items: "nope" }).length).toBe(0);
  });
});

describe("sources/github-billing per-item validation (#2493)", () => {
  test("an impossible date drops only its item; a negative amount contributes zero", () => {
    const days = aggregateBillingUsage([
      { date: "2026-10-01", product: "actions", unitType: "minutes", quantity: 10, grossAmount: 1, netAmount: 1 },
      { date: "2026-02-30", product: "actions", unitType: "minutes", quantity: 99, grossAmount: 9, netAmount: 9 },
      { date: "2026-10-02", product: "actions", unitType: "minutes", quantity: -5, grossAmount: -3, netAmount: -3 },
    ]);
    expect(days.map((d) => d.date)).toEqual(["2026-10-01", "2026-10-02"]);
    expect(days[0].actionsMinutes).toBe(10);
    expect(days[1].actionsMinutes).toBe(0);
    expect(days[1].grossUsd).toBe(0);
  });
});
