// #13261 slice C4.5: gatherer tick end-to-end — a projected
// workhorse full-wall produces one pool_exhaustion record per tick,
// repeat ticks suppress it, and non-workhorse walls stay silent.
import { describe, expect, test } from "bun:test";
import { mkdtemp, readFile, rm, writeFile, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import type { PoolExhaustionCalibrationCell } from "../src/quota-pool-exhaustion.ts";

const GATHERER_SCRIPT = fileURLToPath(new URL("../scripts/gather-quota.ts", import.meta.url));

describe("gatherer tick — pool_exhaustion production + suppression (#13261)", () => {
  test("a projected workhorse pool/window writes one predicted record per tick, repeat ticks do not duplicate", async () => {
    const dir = await mkdtemp(join(tmpdir(), "gather-quota-pool-exhaustion-"));
    try {
      const currentPath = join(dir, "quota-current.json");
      const heartbeatPath = join(dir, "quota-gatherer-heartbeat.json");
      const statePath = join(dir, "quota-gatherer-state.json");
      const snapshotsDir = join(dir, "snapshots");
      const predictionsPath = join(dir, "quota-predictions.jsonl");
      const calibrationPath = join(dir, "quota-exhaustion-calibration.json");
      const alertStatePath = join(dir, "quota-forecast-alert-state.json");
      const subcommandPath = join(dir, "quota-subcommand.mjs");
      const now = Date.now();
      const HOUR = 3_600_000;

      // Weekly window at 70%, reset 10h out: S1 projects a wall ~3h
      // out (10 pts/h measured rate).
      const resetIso = new Date(now + 10 * HOUR).toISOString();
      const seed = [3, 2, 1]
        .map((h) =>
          JSON.stringify({
            ts: new Date(now - h * HOUR).toISOString(),
            serviceId: "opencode",
            metricId: "weekly",
            value: 70 - h * 10,
            unit: "pct",
            limit: 100,
            resetsAt: resetIso,
            source: "api",
            observedBy: "quota-gatherer",
          }),
        )
        .join("\n");
      await mkdir(snapshotsDir, { recursive: true });
      await writeFile(
        join(snapshotsDir, `usage-${new Date(now).toISOString().slice(0, 7)}.jsonl`),
        `${seed}\n`,
        "utf8",
      );

      // Calibration: the S2 ledger's window cell for opencode:weekly
      // has ≥20 scored forecasts → its band calibrates the range.
      // Keyed by poolExhaustionSubjectKey (`opencode:weekly`), the
      // shape gather-quota.ts parses.
      const calibration: Record<string, PoolExhaustionCalibrationCell> = {
        "opencode:weekly": {
          window: { count: 42, lowerMin: -30, upperMin: 45, cell: "opencode:weekly@lead180" },
          pool: null,
        },
      };
      await mkdir(snapshotsDir, { recursive: true });
      await writeFile(calibrationPath, `${JSON.stringify(calibration)}\n`, "utf8");

      const providerResult = {
        providerId: "opencode",
        providerName: "OpenCode",
        ok: true,
        configured: true,
        fetchedAt: now,
        usage: { windows: { weekly: { usedPercent: 70, windowSeconds: 604_800, resetAt: now + 10 * HOUR } } },
      };
      await writeFile(subcommandPath, `console.log(${JSON.stringify(JSON.stringify([providerResult]))});\n`, "utf8");

      const env = {
        PATH: process.env.PATH ?? "",
        HOME: dir,
        QUOTA_CURRENT_PATH: currentPath,
        QUOTA_HEARTBEAT_PATH: heartbeatPath,
        QUOTA_GATHERER_STATE_PATH: statePath,
        QUOTA_SNAPSHOTS_DIR: snapshotsDir,
        QUOTA_SUBCOMMAND: `${process.execPath} ${subcommandPath}`,
        QUOTA_SUBCOMMAND_TIMEOUT_MS: "5000",
        OPENDESIGN_RENEWAL_DISABLED: "1",
        META_RENEWAL_DISABLED: "1",
        TURSO_QUOTA_SINK: "0",
        QUOTA_PREDICTIONS_PATH: predictionsPath,
        QUOTA_FORECAST_ALERT_STATE_PATH: alertStatePath,
      };

      const first = spawnSync(process.execPath, [GATHERER_SCRIPT], {
        cwd: dir,
        encoding: "utf8",
        timeout: 30_000,
        env,
      });
      if (first.error || first.status !== 0) {
        throw new Error(`gatherer tick 1 failed: ${first.error?.message ?? first.stderr}\n${first.stdout}`);
      }
      expect(first.stdout).toContain("pool-exhaustion predictions predicted");

      const log1 = await readFile(predictionsPath, "utf8");
      const records1 = log1
        .trim()
        .split("\n")
        .map((l) => JSON.parse(l));
      const poolRecords = records1.filter((r: { kind: string }) => r.kind === "pool_exhaustion");
      expect(poolRecords).toHaveLength(1);
      const pr = poolRecords[0];
      expect(pr.subject).toEqual({ pool: "opencode", window: "weekly" });
      expect(pr.state).toBe("predicted");
      const forecast = JSON.parse(await readFile(join(dir, "quota-forecast.json"), "utf8"));
      const weekly = forecast.pools["opencode"]?.[0]?.windows?.find((w: { name: string }) => w.name === "weekly");
      expect(weekly?.state).toBe("projected");
      expect(weekly?.wallsBeforeReset).toBe(true);
      const wallMs = Number(weekly.projectedWallAt);
      expect(pr.prediction?.earliest).toBe(new Date(wallMs - 30 * 60_000).toISOString());
      expect(pr.prediction?.latest).toBe(new Date(wallMs + 45 * 60_000).toISOString());
      expect(typeof pr.prediction?.rendered).toBe("string");
      expect(pr.methodology_version).toBe("c4-meth:4");
      expect(pr.supersedes).toBeNull();

      // Repeat tick: same forecast → same record id; the supersedes
      // link chains to it but the log carries each tick's own line,
      // and the alert dedupe (persisted fired state) suppresses the
      // already-alerted finding on repeat. Assert the record itself
      // is re-issued once per tick (idempotent re-issue) and the
      // alert-state does not re-fire.
      const second = spawnSync(process.execPath, [GATHERER_SCRIPT], {
        cwd: dir,
        encoding: "utf8",
        timeout: 30_000,
        env,
      });
      if (second.error || second.status !== 0) {
        throw new Error(`gatherer tick 2 failed: ${second.error?.message ?? second.stderr}\n${second.stdout}`);
      }
      expect(second.stdout).toContain("pool-exhaustion predictions predicted");

      const log2 = await readFile(predictionsPath, "utf8");
      const records2 = log2
        .trim()
        .split("\n")
        .map((l) => JSON.parse(l));
      const poolRecords2 = records2.filter((r: { kind: string }) => r.kind === "pool_exhaustion");
      // One line per tick: each tick re-issues from its own forecast
      // product (new issued_at → new id), chained by supersedes.
      expect(poolRecords2).toHaveLength(2);
      expect(poolRecords2[1].id).not.toBe(poolRecords2[0].id);
      expect(poolRecords2[1].supersedes).toBe(poolRecords2[0].id);
      expect(poolRecords2[1].state).toBe("predicted");
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
