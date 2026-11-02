import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { spawn, type Subprocess } from "bun";

// Contract tests for /api/quota.json hub sections (#1119, epic #1047,
// design hub-v2-v3 §6). Consumers (quota-guidance, the measurement
// gate, the render layer) depend on these shapes; the v2 collector
// work (#1048-#1052) changes producers, and silent schema drift would
// surface as wrong quota guidance, not an error. These tests boot the
// real server on the fixtures source (no secrets, CI-safe) and pin
// the served document's shape per section: required fields, enums,
// and null semantics. They deliberately do NOT pin fixture values.

let proc: Subprocess<"ignore", "pipe", "inherit"> | null = null;
let port = 0;
let doc: Record<string, any>;

const USAGE_UNITS = [
  "pct", "usd", "count", "gb", "tokens", "minutes",
  "hours", "ms", "rows", "ops", "neurons", "other",
];
const SAAS_GROUPS = ["database", "object-storage", "compute", "dev-platform", "domains", "account-tooling"];
const SAAS_STATUSES = ["active", "candidate", "no-account", "unknown"];
const SOURCE_CLASSES = ["api", "console", "manual", "estimate", "invoice"];
const ATTENTION_KINDS = ["over-limit", "payg-unbilled", "unmatched-charge"];

const isNumOrNull = (v: unknown) => v === null || (typeof v === "number" && Number.isFinite(v));
const isStrOrNull = (v: unknown) => v === null || typeof v === "string";

beforeAll(async () => {
  port = 5100 + Math.floor(Math.random() * 1000);
  proc = spawn({
    cmd: ["bun", "run", "src/server.ts"],
    cwd: import.meta.dir + "/..",
    env: { ...Bun.env, PM_AGG_PORT: String(port), PM_AGG_SOURCE: "fixtures" },
    stdout: "pipe",
    stderr: "inherit",
  });
  const deadline = Date.now() + 8000;
  let ready = false;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/api/quota.json`);
      if (res.ok) {
        doc = (await res.json()) as Record<string, any>;
        ready = true;
        break;
      }
    } catch {
      /* not up yet */
    }
    await Bun.sleep(120);
  }
  expect(ready).toBe(true);
});

afterAll(() => {
  proc?.kill();
});

describe("/api/quota.json hub contract (#1119)", () => {
  test("top level: legacy pools shape plus all six hub sections", () => {
    expect(Array.isArray(doc.pools)).toBe(true);
    for (const key of ["saas", "payg", "charges", "attention", "subscriptions"]) {
      expect(Array.isArray(doc[key])).toBe(true);
    }
    expect("domains" in doc).toBe(true); // object or null, never absent
    expect(typeof doc.extractorHealth).toBe("object");
    for (const rows of [doc.extractorHealth?.pools, doc.extractorHealth?.services]) {
      expect(Array.isArray(rows)).toBe(true);
      if (!Array.isArray(rows)) continue;
      for (const row of rows) {
        expect(typeof row.id).toBe("string");
        expect(["matched", "gap", "intentional", "unavailable"]).toContain(row.status);
        expect([true, false, null]).toContain(row.matched);
        expect(typeof row.count).toBe("number");
      }
    }
    expect(doc.extractorHealth.pools.map((row: { id: string }) => row.id)).toEqual(
      doc.pools.map((pool: { id: string }) => pool.id),
    );
    expect(doc.extractorHealth.services.map((row: { id: string }) => row.id)).toEqual(
      doc.saas.map((service: { id: string }) => service.id),
    );
    for (const pool of doc.pools) {
      expect(typeof pool.id).toBe("string");
      expect(Array.isArray(pool.windows)).toBe(true);
    }
  });

  test("saas: service cards carry required fields with enum group/status/sourceClass", () => {
    expect(doc.saas.length).toBeGreaterThan(0);
    for (const svc of doc.saas) {
      expect(typeof svc.id).toBe("string");
      expect(typeof svc.name).toBe("string");
      expect(SAAS_GROUPS).toContain(svc.group);
      expect(typeof svc.plan).toBe("string");
      expect(SAAS_STATUSES).toContain(svc.status);
      expect(Array.isArray(svc.metrics)).toBe(true);
      expect(svc.paygEstimate === null || typeof svc.paygEstimate === "object").toBe(true);
      expect(isStrOrNull(svc.measuredAt)).toBe(true);
      expect(SOURCE_CLASSES).toContain(svc.sourceClass);
      expect(typeof svc.sourceNote).toBe("string");
      expect(typeof svc.cadenceDays).toBe("number");
      expect(Array.isArray(svc.conflicts)).toBe(true);
      expect(Array.isArray(svc.reads)).toBe(true);
    }
  });

  test("saas metrics: unit is always in the UsageUnit enum — there is no cny unit", () => {
    // #1052 (DeepSeek): a CNY balance (e.g. "¥44.29") rides the quota
    // plane as a window valueLabel, never as a metric unit. Pinning
    // the enum here stops a producer from coining unit:"cny" and
    // silently breaking consumers that switch on UsageUnit.
    let seen = 0;
    for (const svc of doc.saas) {
      for (const m of svc.metrics) {
        seen++;
        expect(typeof m.id).toBe("string");
        expect(typeof m.label).toBe("string");
        expect(USAGE_UNITS).toContain(m.unit);
        expect(typeof m.highlight).toBe("boolean");
      }
    }
    expect(seen).toBeGreaterThan(0);
  });

  test("saas metrics: null used means unmeasured — the key is present, never 0-filled, and carries no percent", () => {
    let nullSeen = 0;
    for (const svc of doc.saas) {
      for (const m of svc.metrics) {
        expect("used" in m).toBe(true);
        expect("limit" in m).toBe(true);
        expect(isNumOrNull(m.used)).toBe(true);
        expect(isNumOrNull(m.limit)).toBe(true);
        if (m.used === null) {
          nullSeen++;
          expect(m.percent).toBeUndefined();
        } else {
          expect(m.used).toBeGreaterThanOrEqual(0);
        }
      }
    }
    // The fixtures source exercises the unmeasured case; if a future
    // fixture change measures everything, this guard says so loudly
    // instead of letting the semantics test silently vacate.
    expect(nullSeen).toBeGreaterThan(0);
  });

  test("payg: lines carry id/label/note and number-or-null money fields", () => {
    expect(doc.payg.length).toBeGreaterThan(0);
    for (const line of doc.payg) {
      expect(typeof line.id).toBe("string");
      expect(typeof line.label).toBe("string");
      expect("accruedUsd" in line).toBe(true);
      expect(isNumOrNull(line.accruedUsd)).toBe(true);
      expect("unbilledUsd" in line).toBe(true);
      expect(isNumOrNull(line.unbilledUsd)).toBe(true);
      expect(typeof line.note).toBe("string");
      expect(isStrOrNull(line.measuredAt)).toBe(true);
      expect(SOURCE_CLASSES).toContain(line.sourceClass);
    }
  });

  test("charges: rows are dated, labelled, method/source-attributed, with a numeric USD amount", () => {
    expect(doc.charges.length).toBeGreaterThan(0);
    for (const row of doc.charges) {
      expect(typeof row.at).toBe("string");
      expect(typeof row.label).toBe("string");
      expect(typeof row.amountUsd).toBe("number");
      expect(typeof row.method).toBe("string");
      expect(typeof row.source).toBe("string");
    }
  });

  test("attention: flags use the kind enum with label/detail/href and numeric severity", () => {
    for (const flag of doc.attention) {
      expect(ATTENTION_KINDS).toContain(flag.kind);
      expect(typeof flag.label).toBe("string");
      expect(typeof flag.detail).toBe("string");
      expect(typeof flag.href).toBe("string");
      expect(typeof flag.severity).toBe("number");
    }
  });

  test("domains: null, or the summary triple with a string-or-null next renewal", () => {
    if (doc.domains !== null) {
      expect(typeof doc.domains.totalAnnualUsd).toBe("number");
      expect(typeof doc.domains.domainCount).toBe("number");
      expect(isStrOrNull(doc.domains.nextRenewal)).toBe(true);
    }
  });

  test("subscriptions: flat rows are name/plan strings with a string-or-null renewsAt", () => {
    expect(doc.subscriptions.length).toBeGreaterThan(0);
    for (const sub of doc.subscriptions) {
      expect(typeof sub.name).toBe("string");
      expect(typeof sub.plan).toBe("string");
      expect("renewsAt" in sub).toBe(true);
      expect(isStrOrNull(sub.renewsAt)).toBe(true);
    }
  });
});
