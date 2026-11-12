// Supabase live-export overlay (#1050 / queue task 2985): a fresh
// supabase-export document puts measured used/limit + the measured
// billed state on the Supabase card; a missing, failed or stale
// document reproduces the recorded prose exactly.
import { describe, expect, test } from "bun:test";
import { chmod, mkdir, mkdtemp, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { applySupabaseExport, extractSaasServices, type SupabaseExportDoc } from "../src/saas-extract.ts";
import { SnapshotStore } from "../src/snapshots.ts";
import { liveSource } from "../src/sources/live.ts";
import { createSupabaseClientGetter, resolveSupabaseProjectRef } from "../src/supabase.ts";

const VIEW = `
## Summary (b) — subscriptions, infrastructure, account tooling

| Service | Plan / price | Allowance sketch | Usage / status | Next date recorded |
|---|---|---|---|---|
| Supabase | Free plan (muse-dev2) | DB + egress + storage | manual dashboard read needed | unknown |

## Service details (b)

### Supabase
- **Plan / price:** Free plan (muse-dev2); numerical quotas unknown in local records.
- **Usage / dates:** Not measured — manual dashboard read needed.
- **Last checked:** 2026-10-28.
`;

const NOW = Date.parse("2026-10-29T15:00:00Z");

describe("getSupabaseClient retry-after-null memoization", () => {
  test("retries null, shares the next in-flight attempt, and caches success", async () => {
    const previousProjectRef = process.env.SUPABASE_PROJECT_REF;
    process.env.SUPABASE_PROJECT_REF = "test-project";
    let secretCalls = 0;
    let resolveFirst!: (value: string | null) => void;
    const firstResult = new Promise<string | null>((resolve) => {
      resolveFirst = resolve;
    });
    let resolveRetry!: (value: string | null) => void;
    const retryResult = new Promise<string | null>((resolve) => {
      resolveRetry = resolve;
    });
    const getClient = createSupabaseClientGetter(async () => {
      secretCalls += 1;
      return secretCalls === 1 ? firstResult : retryResult;
    });

    try {
      const initial = getClient();
      const concurrentInitial = getClient();
      expect(secretCalls).toBe(1);
      resolveFirst(null);
      expect(await Promise.all([initial, concurrentInitial])).toEqual([null, null]);

      const first = getClient();
      const second = getClient();
      expect(secretCalls).toBe(2);
      resolveRetry("test-token");

      const [client, concurrentClient] = await Promise.all([first, second]);
      expect(client).not.toBeNull();
      expect(client).toBe(concurrentClient);
      expect(client?.ref).toBe("test-project");
      expect(await getClient()).toBe(client);
      expect(secretCalls).toBe(2);
    } finally {
      if (previousProjectRef === undefined) delete process.env.SUPABASE_PROJECT_REF;
      else process.env.SUPABASE_PROJECT_REF = previousProjectRef;
    }
  });

  test("resolves the project ref through the canonical Bitwarden pointer", async () => {
    const requested: string[] = [];
    const ref = await resolveSupabaseProjectRef(async (key) => {
      requested.push(key);
      return key === "supabase.project-ref" ? "configured-project" : null;
    }, {});

    expect(ref).toBe("configured-project");
    expect(requested).toEqual(["supabase.project-ref"]);
  });

  test("trims and prefers an explicit project-ref override", async () => {
    const ref = await resolveSupabaseProjectRef(async () => {
      throw new Error("secret lookup should not run for an explicit override");
    }, { SUPABASE_PROJECT_REF: "  override-project  " });

    expect(ref).toBe("override-project");
  });

  test("does not fall back to a project literal when the canonical ref is unavailable", async () => {
    const ref = await resolveSupabaseProjectRef(async () => null, {});
    expect(ref).toBeNull();
  });
});

function freshDoc(overrides: Partial<SupabaseExportDoc> = {}): SupabaseExportDoc {
  return {
    service: "supabase",
    source: "file-export/supabase",
    status: "partial",
    measuredAt: "2026-10-29T14:55:00Z",
    project: "muse-dev2",
    ref: "fixture-project-ref",
    metrics: [
      {
        id: "db-size",
        label: "Database size",
        used: 1.154,
        limit: 0.5,
        unit: "gb",
        source: "management-api:/database/query pg_database_size",
        flag: "approaching-paid-trigger",
      },
      {
        id: "egress",
        label: "Egress (cycle)",
        used: null,
        limit: 5,
        unit: "gb",
        source: "none",
        detail: "No Management API endpoint reports egress bytes.",
      },
      {
        id: "storage",
        label: "File storage",
        used: 0,
        limit: 1,
        unit: "gb",
        source: "management-api:/storage/buckets",
        detail: "0 buckets on the project",
      },
    ],
    billing: { plan: "free", billedUsd: 0, measured: true, source: "management-api:/billing/addons" },
    scope: [
      { endpoint: "projects", status: 200 },
      { endpoint: "organizations/:slug", status: 403, note: "scope gap" },
      { endpoint: "usage.egress", status: 404 },
    ],
    ...overrides,
  };
}

const supabaseCard = (services: ReturnType<typeof extractSaasServices>) => services.find((s) => s.id === "supabase")!;
const metricOf = (services: ReturnType<typeof extractSaasServices>, id: string) =>
  supabaseCard(services).metrics.find((m) => m.id === id)!;

describe("applySupabaseExport", () => {
  test("a fresh export supplies measured used + Free limits for db-size and storage", () => {
    const out = applySupabaseExport(extractSaasServices(VIEW), freshDoc(), NOW);
    const db = metricOf(out, "db-size");
    expect(db.used).toBe(1.154);
    expect(db.limit).toBe(0.5);
    expect(db.unit).toBe("gb");
    expect(db.percent).toBe(230.8);
    expect(db.note).toContain("measured 2026-10-29T14:55:00Z");
    expect(db.note).toContain("Approaching-paid trigger");
    const storage = metricOf(out, "storage");
    expect(storage.used).toBe(0);
    expect(storage.limit).toBe(1);
    expect(supabaseCard(out).sourceClass).toBe("api");
    expect(supabaseCard(out).measuredAt).toBe("2026-10-29");
  });

  test("egress keeps a recorded limit but stays unmeasured, with the gap detail", () => {
    const out = applySupabaseExport(extractSaasServices(VIEW), freshDoc(), NOW);
    const egress = metricOf(out, "egress");
    expect(egress.used).toBeNull();
    expect(egress.limit).toBe(5);
    expect(egress.percent).toBeUndefined();
    expect(egress.note).toContain("Not measured this tick");
    expect(egress.note).toContain("egress bytes");
  });

  test("billed is the export's measured addon read, not the hardcoded 0", () => {
    const out = applySupabaseExport(
      extractSaasServices(VIEW),
      freshDoc({ billing: { plan: "free", billedUsd: 25, measured: true } }),
      NOW,
    );
    expect(supabaseCard(out).billedUsd).toBe(25);
    expect(supabaseCard(out).plan).toContain("measured 2026-10-29T14:55:00Z");
  });

  test("an accepted export without a valid billing measurement clears billed to unknown", () => {
    const noBilling = applySupabaseExport(
      extractSaasServices(VIEW),
      freshDoc({ billing: null as unknown as SupabaseExportDoc["billing"] }),
      NOW,
    );
    expect(supabaseCard(noBilling).billedUsd).toBeNull();
    expect(supabaseCard(noBilling).sourceClass).toBe("api"); // the rest of the overlay still applied
    expect(metricOf(noBilling, "db-size").used).toBe(1.154);
    const unmeasured = applySupabaseExport(extractSaasServices(VIEW), freshDoc({ billing: { plan: "free" } }), NOW);
    expect(supabaseCard(unmeasured).billedUsd).toBeNull();
  });

  test("negative billing is unknown without discarding valid metric measurements", () => {
    const out = applySupabaseExport(
      extractSaasServices(VIEW),
      freshDoc({ billing: { plan: "free", billedUsd: -42.5, measured: true } }),
      NOW,
    );
    expect(supabaseCard(out).billedUsd).toBeNull();
    expect(supabaseCard(out).sourceClass).toBe("api");
    expect(metricOf(out, "db-size").used).toBe(1.154);
  });

  test("a non-string measuredAt is a no-op before Date.parse coercion", () => {
    const base = extractSaasServices(VIEW);
    const expected = structuredClone(base);
    const out = applySupabaseExport(base, freshDoc({ measuredAt: ["2026-10-29T14:55:00Z"] as unknown as string }), NOW);
    expect(out).toEqual(expected);
  });

  test("a negative used value leaves that metric at its recorded fallback", () => {
    const base = extractSaasServices(VIEW);
    const recordedDb = structuredClone(metricOf(base, "db-size"));
    const out = applySupabaseExport(
      base,
      freshDoc({
        metrics: [
          { id: "db-size", used: -3.25, limit: 0.5 },
          { id: "storage", used: 0.25, limit: 1, unit: "gb" },
        ],
      }),
      NOW,
    );
    expect(metricOf(out, "db-size")).toEqual(recordedDb);
    expect(metricOf(out, "storage").used).toBe(0.25);
  });

  test("a negative limit leaves that metric at its recorded fallback", () => {
    const base = extractSaasServices(VIEW);
    const recordedDb = structuredClone(metricOf(base, "db-size"));
    const out = applySupabaseExport(base, freshDoc({ metrics: [{ id: "db-size", used: 0.25, limit: -0.5 }] }), NOW);
    expect(metricOf(out, "db-size")).toEqual(recordedDb);
  });

  test("an impossible or non-ISO measuredAt changes nothing — no Date.parse rollover (#5018)", () => {
    // now is 2026-03-02T12:05Z: Date.parse rolls 2026-02-30 over to
    // Mar 2 12:00, inside the freshness window; the real-calendar gate
    // refuses it, and non-ISO stamps are refused the same way.
    const now = Date.parse("2026-03-02T12:05:00Z");
    for (const bad of ["2026-02-30T12:00:00.000Z", "2026-04-31T12:00:00.000Z", "March 2, 2026 12:00 UTC"]) {
      const base = extractSaasServices(VIEW);
      const out = applySupabaseExport(base, freshDoc({ measuredAt: bad }), now);
      expect(out).toEqual(base);
      expect(supabaseCard(out).sourceClass).toBe("manual");
    }
  });

  test("a leap-date measuredAt still overlays (#5018)", () => {
    const now = Date.parse("2024-02-29T12:05:00Z");
    const base = extractSaasServices(VIEW);
    const out = applySupabaseExport(base, freshDoc({ measuredAt: "2024-02-29T12:00:00.000Z" }), now);
    expect(metricOf(out, "db-size").used).toBe(1.154);
    expect(supabaseCard(out).sourceClass).toBe("api");
    expect(supabaseCard(out).measuredAt).toBe("2024-02-29");
  });

  test("a stale export falls back to exactly today's card", () => {
    const base = extractSaasServices(VIEW);
    const out = applySupabaseExport(base, freshDoc({ measuredAt: "2026-10-29T12:00:00Z" }), NOW);
    expect(out).toEqual(base);
    expect(supabaseCard(out).billedUsd).toBe(0); // today's constant
    expect(metricOf(out, "db-size").used).toBeNull();
  });

  test("missing, failed and unparseable documents change nothing", () => {
    const base = extractSaasServices(VIEW);
    expect(applySupabaseExport(base, null, NOW)).toEqual(base);
    expect(applySupabaseExport(base, freshDoc({ status: "fetch-failed", metrics: undefined }), NOW)).toEqual(base);
    expect(applySupabaseExport(base, freshDoc({ measuredAt: "not-a-date" }), NOW)).toEqual(base);
    expect(applySupabaseExport(base, freshDoc({ metrics: [] }), NOW)).toEqual(base);
  });

  test("a document without a Supabase card in the view is a no-op", () => {
    const out = applySupabaseExport(extractSaasServices("nothing here"), freshDoc(), NOW);
    expect(out.find((s) => s.id === "supabase")!.metrics).toEqual([]);
  });
});

describe("live source usage(): the overlay reaches fleet-feed services too", () => {
  test("feed.saas cards get the fresh export overlay, not just the quota-view fallback", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-supa-feed-"));
    const nowIso = new Date().toISOString();
    const feedFile = join(dir, "fleet-feed.json");
    const exportFile = join(dir, "latest.json");
    await writeFile(exportFile, JSON.stringify(freshDoc({ measuredAt: nowIso })));
    await writeFile(
      feedFile,
      JSON.stringify({
        version: 1,
        generatedAt: nowIso,
        quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
        attention: [],
        saas: extractSaasServices(VIEW),
      }),
    );
    const cfg = loadConfig({
      PM_AGG_SOURCE: "live",
      PM_AGG_FEED_FILE: feedFile,
      PM_AGG_SUPABASE_EXPORT: exportFile,
      PM_AGG_REGISTRY_FILE: join(dir, "registry.json"),
      PM_AGG_COMMENTS_FILE: join(dir, "comments.json"),
      PM_AGG_SNAPSHOTS_DIR: join(dir, "snapshots"),
      PM_AGG_OPENCHAMBER_STATE: join(dir, "no-state.json"),
      PM_PROJECTOR_URL: "off",
      KANEO_ADMIN_EMAIL: "",
      KANEO_ADMIN_PASSWORD: "",
      GRIST_API_KEY: "",
    });
    const usage = await liveSource(cfg).usage();
    const db = metricOf(usage.services, "db-size");
    expect(db.used).toBe(1.154);
    expect(db.limit).toBe(0.5);
    expect(supabaseCard(usage.services).billedUsd).toBe(0);
  });

  test("usage reads feed history without writing to a read-only snapshot store", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-supa-readonly-"));
    const feedFile = join(dir, "fleet-feed.json");
    const snapshotsDir = join(dir, "snapshots");
    const nowIso = new Date().toISOString();
    const record = {
      ts: nowIso,
      serviceId: "supabase",
      metricId: "db-size",
      value: 1.154,
      unit: "gb",
      source: "api",
      observedBy: "fleet-feed-collector",
    };
    try {
      await mkdir(snapshotsDir);
      await chmod(snapshotsDir, 0o555);
      await writeFile(
        feedFile,
        JSON.stringify({
          version: 1,
          generatedAt: nowIso,
          quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
          attention: [],
          saas: extractSaasServices(VIEW),
          snapshots: [record],
        }),
      );
      const cfg = loadConfig({
        PM_AGG_SOURCE: "live",
        PM_AGG_FEED_FILE: feedFile,
        PM_AGG_SNAPSHOTS_DIR: snapshotsDir,
        PM_AGG_SUPABASE_EXPORT: join(dir, "latest.json"),
        PM_AGG_REGISTRY_FILE: join(dir, "registry.json"),
        PM_AGG_COMMENTS_FILE: join(dir, "comments.json"),
        PM_AGG_OPENCHAMBER_STATE: join(dir, "no-state.json"),
        PM_PROJECTOR_URL: "off",
        KANEO_ADMIN_EMAIL: "",
        KANEO_ADMIN_PASSWORD: "",
        GRIST_API_KEY: "",
      });
      const usage = await liveSource(cfg).usage();
      expect(usage.services.some((service) => service.id === "supabase")).toBe(true);
      expect(await readdir(snapshotsDir)).toEqual([]);
    } finally {
      await chmod(snapshotsDir, 0o755).catch(() => {});
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("background ingestion lands feed records idempotently", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-supa-ingest-"));
    const feedFile = join(dir, "fleet-feed.json");
    const snapshotsDir = join(dir, "snapshots");
    const nowIso = new Date().toISOString();
    const record = {
      ts: nowIso,
      serviceId: "supabase",
      metricId: "db-size",
      value: 1.154,
      unit: "gb",
      source: "api",
      observedBy: "fleet-feed-collector",
    };
    try {
      await writeFile(
        feedFile,
        JSON.stringify({
          version: 1,
          generatedAt: nowIso,
          quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
          attention: [],
          saas: extractSaasServices(VIEW),
          snapshots: [record],
        }),
      );
      const cfg = loadConfig({
        PM_AGG_SOURCE: "live",
        PM_AGG_FEED_FILE: feedFile,
        PM_AGG_SNAPSHOTS_DIR: snapshotsDir,
        PM_AGG_SUPABASE_EXPORT: join(dir, "latest.json"),
        PM_AGG_REGISTRY_FILE: join(dir, "registry.json"),
        PM_AGG_COMMENTS_FILE: join(dir, "comments.json"),
        PM_AGG_OPENCHAMBER_STATE: join(dir, "no-state.json"),
        PM_PROJECTOR_URL: "off",
        KANEO_ADMIN_EMAIL: "",
        KANEO_ADMIN_PASSWORD: "",
        GRIST_API_KEY: "",
      });
      const source = liveSource(cfg);
      expect(await source.ingestSnapshots?.()).toBe(1);
      expect(await source.ingestSnapshots?.()).toBe(0);
      expect(await new SnapshotStore(snapshotsDir).byService()).toMatchObject({ supabase: [record] });
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
