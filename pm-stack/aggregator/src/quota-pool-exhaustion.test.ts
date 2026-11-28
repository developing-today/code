// C4.5 pool_exhaustion producer tests (#13261, component of the C4
// quota-forecasting epic #9572). Oracle: the C4 design
// doc/2026-11-13T15-12-00Z_design_reporting-architecture-event-driven/
// c4-forecasting-design.md §1 (record shape, honesty contract) and
// §2 (pool_exhaustion: likely = projected wall, range calibrated by
// the S2 accuracy record; window cell <20 falls back to the pool's
// all-window band; below that, insufficient_history).

import { describe, expect, test } from "bun:test";
import type { QuotaForecastFile } from "./quota-forecast.ts";
import {
  buildPoolExhaustionLedgerRows,
  derivePoolExhaustionRecords,
  poolExhaustionSubjectKey,
  type PoolExhaustionCalibration,
} from "./quota-pool-exhaustion.ts";
const ISSUED = "2026-11-20T12:00:00.000Z";
const ISSUED_MS = Date.parse(ISSUED);
const HOUR = 3_600_000;

function forecastWithWindow(w: Record<string, unknown>): QuotaForecastFile {
  return {
    version: 1,
    generatedAt: ISSUED,
    source: "gather-quota",
    pools: {
      opencode: [
        {
          providerId: "opencode",
          serviceId: "opencode-go",
          plan: null,
          windows: [
            {
              name: "weekly",
              state: "projected",
              ratePtsPerHour: 10,
              samples: 12,
              projectedWallAt: ISSUED_MS + 3 * HOUR,
              durationUntilWallMs: 3 * HOUR,
              wallsBeforeReset: true,
              pctNow: 70,
              measuredAt: new Date(ISSUED_MS - HOUR).toISOString(),
              resetsAt: new Date(ISSUED_MS + 10 * HOUR).toISOString(),
              ...w,
            } as never,
          ],
        },
      ],
    },
  };
}

const WINDOW_BAND = { count: 42, lowerMin: -30, upperMin: 45, cell: "opencode-go:weekly@lead180" };

function calibrationWith(window: typeof WINDOW_BAND | null, poolCount = 0): PoolExhaustionCalibration {
  return new Map([
    [
      poolExhaustionSubjectKey("opencode", "weekly"),
      {
        window,
        pool: poolCount > 0 ? { count: poolCount, lowerMin: -60, upperMin: 90, cell: "opencode-go:all-windows" } : null,
      },
    ],
  ]);
}

describe("derivePoolExhaustionRecords — live projection maps to a predicted record", () => {
  test("likely is the S1 projected wall, ranged by the window cell", () => {
    const [r] = derivePoolExhaustionRecords(forecastWithWindow({}), calibrationWith(WINDOW_BAND));
    expect(r).toBeDefined();
    expect(r.kind).toBe("pool_exhaustion");
    expect(r.state).toBe("predicted");
    expect(r.subject).toEqual({ pool: "opencode", window: "weekly" });
    expect(r.prediction?.earliest).toBe(new Date(ISSUED_MS + 3 * HOUR - 30 * 60_000).toISOString());
    expect(r.prediction?.latest).toBe(new Date(ISSUED_MS + 3 * HOUR + 45 * 60_000).toISOString());
    expect(typeof r.prediction?.rendered).toBe("string");
    expect(r.prediction?.rendered.length).toBeGreaterThan(0);
    expect(r.basis.name).toBe("since_last_reset");
    expect(r.basis.detail).toContain("opencode-go:weekly@lead180");
    expect(r.methodology_version).toBe("c4-meth:4");
    expect(r.id).toBe(`c4:pool_exhaustion:opencode:weekly:${ISSUED_MS}`);
  });
});

describe("derivePoolExhaustionRecords — small window cell falls back to the pool band", () => {
  test("window n<20 uses the pool all-window band and names it", () => {
    const [r] = derivePoolExhaustionRecords(
      forecastWithWindow({}),
      calibrationWith({ count: 7, lowerMin: -30, upperMin: 45, cell: "opencode-go:weekly@lead180" }, 63),
    );
    expect(r.state).toBe("predicted");
    expect(r.prediction?.earliest).toBe(new Date(ISSUED_MS + 3 * HOUR - 60 * 60_000).toISOString());
    expect(r.prediction?.latest).toBe(new Date(ISSUED_MS + 3 * HOUR + 90 * 60_000).toISOString());
    expect(r.basis.detail).toContain("opencode-go:all-windows");
  });
});

describe("derivePoolExhaustionRecords — honesty when no band is earned", () => {
  test("no calibration at all publishes insufficient_history, never a range", () => {
    const [r] = derivePoolExhaustionRecords(forecastWithWindow({}), new Map());
    expect(r.state).toBe("insufficient_history");
    expect(r.prediction).toBeNull();
    expect(r.state_reason).toContain("no accuracy cell");
    expect(r.id).toContain("@honesty:");
  });

  test("thin window cell and thin pool band publish insufficient_history", () => {
    const [r] = derivePoolExhaustionRecords(
      forecastWithWindow({}),
      calibrationWith({ count: 3, lowerMin: -30, upperMin: 45, cell: "opencode-go:weekly@lead180" }, 9),
    );
    expect(r.state).toBe("insufficient_history");
    expect(r.prediction).toBeNull();
  });
});

describe("derivePoolExhaustionRecords — subjects", () => {
  test("walled windows record the observed wall as likely", () => {
    const [r] = derivePoolExhaustionRecords(
      forecastWithWindow({ state: "walled", projectedWallAt: null, durationUntilWallMs: 0, wallsBeforeReset: true }),
      calibrationWith(WINDOW_BAND),
    );
    expect(r.state).toBe("predicted");
    expect(r.basis.detail).toContain("already walled at issuance");
  });

  test("windows not projected to wall before reset publish nothing", () => {
    expect(
      derivePoolExhaustionRecords(forecastWithWindow({ state: "no-wall-before-reset" }), calibrationWith(WINDOW_BAND)),
    ).toEqual([]);
    expect(
      derivePoolExhaustionRecords(
        forecastWithWindow({ state: "projected", wallsBeforeReset: false }),
        calibrationWith(WINDOW_BAND),
      ),
    ).toEqual([]);
    expect(
      derivePoolExhaustionRecords(forecastWithWindow({ state: "insufficient-history" }), calibrationWith(WINDOW_BAND)),
    ).toEqual([]);
  });

  test("split pools yield one record per window — the first row wins", () => {
    const file = forecastWithWindow({});
    file.pools["opencode"]!.push({ ...file.pools["opencode"]![0]! });
    const records = derivePoolExhaustionRecords(file, calibrationWith(WINDOW_BAND));
    expect(records).toHaveLength(1);
  });

  test("supersedes links to the previous record for the subject", () => {
    const previous = new Map([
      [poolExhaustionSubjectKey("opencode", "weekly"), "c4:pool_exhaustion:opencode:weekly:1"],
    ]);
    const [r] = derivePoolExhaustionRecords(forecastWithWindow({}), calibrationWith(WINDOW_BAND), previous);
    expect(r.supersedes).toBe("c4:pool_exhaustion:opencode:weekly:1");
  });
});

describe("buildPoolExhaustionLedgerRows", () => {
  test("one row per record under the C4 source with the latest bound as resets", () => {
    const records = derivePoolExhaustionRecords(forecastWithWindow({}), calibrationWith(WINDOW_BAND));
    const rows = buildPoolExhaustionLedgerRows(records, ISSUED);
    expect(rows).toHaveLength(1);
    expect(rows[0].source).toBe("quota-prediction");
    expect(rows[0]?.resets).toBe(records[0]?.prediction?.latest ?? null);
    expect((rows[0]?.payload as { id: string }).id).toBe(records[0]?.id);
  });
});
