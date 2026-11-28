// Live source usage(): fleet-feed snapshot history is read without
// writing to the snapshot store, and background ingestion lands feed
// records idempotently. (Moved here from the retired Supabase-export
// suite when the export overlay was removed; the behaviour is generic.)
import { describe, expect, test } from "bun:test";
import { chmod, mkdir, mkdtemp, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { extractSaasServices } from "../src/saas-extract.ts";
import { SnapshotStore } from "../src/snapshots.ts";
import { liveSource } from "../src/sources/live.ts";

const VIEW = `
## Summary (b) — subscriptions, infrastructure, account tooling

| Service | Plan / price | Allowance sketch | Usage / status | Next date recorded |
|---|---|---|---|---|
| Turso | Starter Free | DB + rows read/written | manual dashboard read needed | unknown |

## Service details (b)

### Turso
- **Plan / price:** Starter Free.
- **Usage / dates:** Not measured — manual dashboard read needed.
- **Last checked:** 2026-10-28.
`;

function liveConfig(dir: string, feedFile: string, snapshotsDir: string) {
  return loadConfig({
    PM_AGG_SOURCE: "live",
    PM_AGG_FEED_FILE: feedFile,
    PM_AGG_SNAPSHOTS_DIR: snapshotsDir,
    PM_AGG_REGISTRY_FILE: join(dir, "registry.json"),
    PM_AGG_COMMENTS_FILE: join(dir, "comments.json"),
    PM_AGG_OPENCHAMBER_STATE: join(dir, "no-state.json"),
    PM_PROJECTOR_URL: "off",
    KANEO_ADMIN_EMAIL: "",
    KANEO_ADMIN_PASSWORD: "",
    GRIST_API_KEY: "",
  });
}

function feedWith(nowIso: string, record: Record<string, unknown>): string {
  return JSON.stringify({
    version: 1,
    generatedAt: nowIso,
    quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
    attention: [],
    saas: extractSaasServices(VIEW),
    snapshots: [record],
  });
}

function snapshotRecord(nowIso: string) {
  return {
    ts: nowIso,
    serviceId: "turso",
    metricId: "rows-read",
    value: 1154,
    unit: "count",
    source: "api",
    observedBy: "fleet-feed-collector",
  };
}

describe("live source usage(): fleet-feed snapshot history", () => {
  test("usage reads feed history without writing to a read-only snapshot store", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-usage-readonly-"));
    const feedFile = join(dir, "fleet-feed.json");
    const snapshotsDir = join(dir, "snapshots");
    const nowIso = new Date().toISOString();
    try {
      await mkdir(snapshotsDir);
      await chmod(snapshotsDir, 0o555);
      await writeFile(feedFile, feedWith(nowIso, snapshotRecord(nowIso)));
      const usage = await liveSource(liveConfig(dir, feedFile, snapshotsDir)).usage();
      expect(usage.services.some((service) => service.id === "turso")).toBe(true);
      expect(await readdir(snapshotsDir)).toEqual([]);
    } finally {
      await chmod(snapshotsDir, 0o755).catch(() => {});
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("background ingestion lands feed records idempotently", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-usage-ingest-"));
    const feedFile = join(dir, "fleet-feed.json");
    const snapshotsDir = join(dir, "snapshots");
    const nowIso = new Date().toISOString();
    const record = snapshotRecord(nowIso);
    try {
      await writeFile(feedFile, feedWith(nowIso, record));
      const source = liveSource(liveConfig(dir, feedFile, snapshotsDir));
      expect(await source.ingestSnapshots?.()).toBe(1);
      expect(await source.ingestSnapshots?.()).toBe(0);
      expect(await new SnapshotStore(snapshotsDir).byService()).toMatchObject({ turso: [record] });
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
