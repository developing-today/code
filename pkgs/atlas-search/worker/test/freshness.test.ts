import { describe, expect, it } from "vitest";
import {
  DEFAULT_FRESHNESS_CONFIG,
  evaluateFreshness,
  evaluateWithWatermarkRead,
  readTenantWatermark,
  resolveFreshnessConfig,
  type CacheEntryFacts,
  type DataClass,
  type FreshnessConfig,
  type FreshnessContext,
} from "../src/freshness";

// Oracle for every test in this file: design #9134 §1 (merged via
// PR #13098). A cached entry may be served only inside its freshness
// bound; the stale-serves counter's expected value is zero.

const T0 = 1_700_000_000;
const OK_W0: FreshnessContext["watermark"] = { status: "ok", currentWatermark: "w0" };
const DOWN: FreshnessContext["watermark"] = { status: "unavailable" };

function entry(dataClass: DataClass, over: Partial<CacheEntryFacts> = {}): CacheEntryFacts {
  return { dataClass, sourceWatermark: "w0", filledAt: T0, ...over };
}

describe("Class A — the hash is the version", () => {
  it("serves a hash match at any age, watermark state irrelevant", () => {
    const e = entry("A", { contentHash: "h1" });
    const v = evaluateFreshness(e, { now: T0 + 10_000_000, watermark: DOWN, requestedContentHash: "h1" });
    expect(v.fresh).toBe(true);
    expect(v.reason).toBe("content-hash-match");
    expect(v.boundSeconds).toBeNull();
  });

  it("never serves a hash mismatch", () => {
    const e = entry("A", { contentHash: "h1" });
    const v = evaluateFreshness(e, { now: T0, watermark: OK_W0, requestedContentHash: "h2" });
    expect(v.fresh).toBe(false);
    expect(v.reason).toBe("content-hash-mismatch");
  });

  it("cannot prove identity when the entry carries no hash", () => {
    const v = evaluateFreshness(entry("A"), { now: T0, watermark: OK_W0, requestedContentHash: "h1" });
    expect(v.fresh).toBe(false);
    expect(v.reason).toBe("content-hash-mismatch");
  });

  it("serves without a requested hash (the §5 key is the hash)", () => {
    const v = evaluateFreshness(entry("A", { contentHash: "h1" }), { now: T0 + 999, watermark: DOWN });
    expect(v.fresh).toBe(true);
  });
});

describe("Class B — watermark equality or TTL_B, whichever fails first", () => {
  it("serves inside the TTL while the watermark is current", () => {
    const v = evaluateFreshness(entry("B"), { now: T0 + 60, watermark: OK_W0 });
    expect(v.fresh).toBe(true);
    expect(v.reason).toBe("watermark-current");
    expect(v.boundSeconds).toBe(60);
  });

  it("a watermark advance makes the entry stale even inside the TTL", () => {
    const v = evaluateFreshness(entry("B"), {
      now: T0 + 5,
      watermark: { status: "ok", currentWatermark: "w1" },
    });
    expect(v.fresh).toBe(false);
    expect(v.reason).toBe("watermark-advanced");
  });

  it("expires on TTL even while the watermark is current", () => {
    const v = evaluateFreshness(entry("B"), { now: T0 + 61, watermark: OK_W0 });
    expect(v.fresh).toBe(false);
    expect(v.reason).toBe("ttl-expired");
  });

  it("degrades to TTL-only halved when the watermark source is unreachable", () => {
    const inside = evaluateFreshness(entry("B"), { now: T0 + 30, watermark: DOWN });
    expect(inside.fresh).toBe(true);
    expect(inside.reason).toBe("degraded-ttl");
    expect(inside.boundSeconds).toBe(30);
    expect(inside.degraded).toBe(true);
    const outside = evaluateFreshness(entry("B"), { now: T0 + 31, watermark: DOWN });
    expect(outside.fresh).toBe(false);
    expect(outside.reason).toBe("ttl-expired");
  });

  it("degradation never extends: 45 s is stale degraded though inside the normal TTL", () => {
    const v = evaluateFreshness(entry("B"), { now: T0 + 45, watermark: DOWN });
    expect(v.fresh).toBe(false);
  });
});

describe("Class C — TTL_C and no later invalidation event", () => {
  it("serves inside TTL_C with no invalidation event", () => {
    const v = evaluateFreshness(entry("C"), { now: T0 + 900, watermark: OK_W0, lastInvalidationAt: null });
    expect(v.fresh).toBe(true);
    expect(v.reason).toBe("within-ttl");
    expect(v.boundSeconds).toBe(900);
  });

  it("expires past TTL_C", () => {
    const v = evaluateFreshness(entry("C"), { now: T0 + 901, watermark: OK_W0 });
    expect(v.fresh).toBe(false);
    expect(v.reason).toBe("ttl-expired");
  });

  it("an invalidation event that post-dates the entry makes it stale", () => {
    const v = evaluateFreshness(entry("C"), { now: T0 + 10, watermark: OK_W0, lastInvalidationAt: T0 + 1 });
    expect(v.fresh).toBe(false);
    expect(v.reason).toBe("invalidated");
  });

  it("an event at or before fill time does not invalidate", () => {
    const at = evaluateFreshness(entry("C"), { now: T0 + 10, watermark: OK_W0, lastInvalidationAt: T0 });
    expect(at.fresh).toBe(true);
    const before = evaluateFreshness(entry("C"), { now: T0 + 10, watermark: OK_W0, lastInvalidationAt: T0 - 5 });
    expect(before.fresh).toBe(true);
  });

  it("stops being served entirely while the watermark source is unreachable", () => {
    const v = evaluateFreshness(entry("C"), { now: T0, watermark: DOWN });
    expect(v.fresh).toBe(false);
    expect(v.reason).toBe("watermark-unavailable");
  });
});

describe("fail-safe edges", () => {
  it("a fill time in the future is never fresh for the TTL classes", () => {
    const b = evaluateFreshness(entry("B", { filledAt: T0 + 100 }), { now: T0, watermark: OK_W0 });
    expect(b.fresh).toBe(false);
    expect(b.reason).toBe("clock-skew");
    const c = evaluateFreshness(entry("C", { filledAt: T0 + 100 }), { now: T0, watermark: OK_W0 });
    expect(c.fresh).toBe(false);
    expect(c.reason).toBe("clock-skew");
  });
});

describe("config resolution — a bad value never widens a bound", () => {
  it("defaults are the design's: TTL_B 60 s, TTL_C 15 min, divisor 2", () => {
    expect(DEFAULT_FRESHNESS_CONFIG).toEqual({ ttlBSeconds: 60, ttlCSeconds: 900, degradedDivisor: 2 });
    expect(resolveFreshnessConfig()).toEqual(DEFAULT_FRESHNESS_CONFIG);
  });

  it("honors valid overrides", () => {
    const cfg = resolveFreshnessConfig({ ttlBSeconds: 10, ttlCSeconds: 60, degradedDivisor: 4 });
    expect(cfg).toEqual({ ttlBSeconds: 10, ttlCSeconds: 60, degradedDivisor: 4 });
    const v = evaluateFreshness(entry("B"), { now: T0 + 11, watermark: OK_W0 }, cfg);
    expect(v.fresh).toBe(false);
  });

  it("falls back to defaults for non-positive or non-finite overrides", () => {
    const cfg = resolveFreshnessConfig({ ttlBSeconds: -5, ttlCSeconds: Number.NaN, degradedDivisor: 0 });
    expect(cfg).toEqual(DEFAULT_FRESHNESS_CONFIG);
  });

  it("clamps a sub-1 divisor so degradation can never extend TTL_B", () => {
    const cfg = resolveFreshnessConfig({ degradedDivisor: 0.5 });
    const v = evaluateFreshness(entry("B"), { now: T0 + 45, watermark: DOWN }, cfg);
    expect(v.boundSeconds).not.toBeNull();
    expect(v.boundSeconds as number).toBeLessThanOrEqual(cfg.ttlBSeconds);
    expect(v.fresh).toBe(true); // bound is TTL_B itself, not beyond it
    const beyond = evaluateFreshness(entry("B"), { now: T0 + 61, watermark: DOWN }, cfg);
    expect(beyond.fresh).toBe(false);
  });
});

describe("the tenant watermark read", () => {
  it("returns the watermark verbatim", async () => {
    expect(await readTenantWatermark(async () => "w7", "tenant-1")).toEqual({
      status: "ok",
      currentWatermark: "w7",
    });
  });

  it("maps null and throws to unavailable — never to a guessed watermark", async () => {
    expect(await readTenantWatermark(async () => null, "tenant-1")).toEqual({ status: "unavailable" });
    expect(
      await readTenantWatermark(async () => {
        throw new Error("turso unreachable");
      }, "tenant-1"),
    ).toEqual({ status: "unavailable" });
  });

  it("evaluateWithWatermarkRead composes the read and the pure rules", async () => {
    const advanced = await evaluateWithWatermarkRead(entry("B"), "tenant-1", async () => "w9", { now: T0 + 5 });
    expect(advanced.fresh).toBe(false);
    expect(advanced.reason).toBe("watermark-advanced");
    const degraded = await evaluateWithWatermarkRead(
      entry("B"),
      "tenant-1",
      async () => {
        throw new Error("down");
      },
      { now: T0 + 30 },
    );
    expect(degraded.fresh).toBe(true);
    expect(degraded.reason).toBe("degraded-ttl");
  });
});

/**
 * The §1 model, transcribed directly from the design table as an
 * independent predicate: true iff a serve at this moment is inside
 * the bound. The property oracle below holds the evaluator to exact
 * agreement with this transcription over generated sequences.
 */
function withinBoundModel(e: CacheEntryFacts, ctx: FreshnessContext, cfg: FreshnessConfig): boolean {
  const age = ctx.now - e.filledAt;
  switch (e.dataClass) {
    case "A":
      return ctx.requestedContentHash === undefined || e.contentHash === ctx.requestedContentHash;
    case "B": {
      if (age < 0) return false;
      if (ctx.watermark.status === "unavailable") {
        return age <= Math.floor(cfg.ttlBSeconds / Math.max(1, cfg.degradedDivisor));
      }
      return e.sourceWatermark === ctx.watermark.currentWatermark && age <= cfg.ttlBSeconds;
    }
    case "C": {
      if (age < 0) return false;
      if (ctx.watermark.status === "unavailable") return false;
      if (ctx.lastInvalidationAt != null && ctx.lastInvalidationAt > e.filledAt) return false;
      return age <= cfg.ttlCSeconds;
    }
  }
}

/** Deterministic PRNG (mulberry32) — generated cases must reproduce. */
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

describe("property oracle — no serve happens outside the bound", () => {
  it("evaluator agrees with the §1 model over generated (watermark, age, class, event) sequences; stale-serves counter is zero", () => {
    const configs: FreshnessConfig[] = [
      DEFAULT_FRESHNESS_CONFIG,
      resolveFreshnessConfig({ ttlBSeconds: 10, ttlCSeconds: 120, degradedDivisor: 4 }),
      resolveFreshnessConfig({ ttlBSeconds: 300, ttlCSeconds: 60, degradedDivisor: 1 }),
    ];
    const classes: DataClass[] = ["A", "B", "C"];
    let staleServes = 0; // evaluator served outside the bound — §7's counter, expected zero
    let missedServes = 0; // evaluator refused inside the bound — over-narrowing is also a defect
    let evaluations = 0;

    for (let seq = 0; seq < 200; seq++) {
      const rand = mulberry32(0x9134 + seq);
      const cfg = configs[seq % configs.length];
      const dataClass = classes[Math.floor(rand() * classes.length)];
      const watermarks = ["w0", "w1", "w2", "w3"];
      const hashes = ["h0", "h1", "h2"];
      let watermarkIdx = Math.floor(rand() * 2); // entry fills at w0 or w1
      const e: CacheEntryFacts = {
        dataClass,
        sourceWatermark: watermarks[watermarkIdx],
        filledAt: T0,
        contentHash: hashes[Math.floor(rand() * hashes.length)],
      };
      let now = T0;
      let sourceDown = false;
      let lastInvalidationAt: number | null = null;
      let requestedContentHash: string | undefined =
        dataClass === "A" && rand() < 0.7 ? e.contentHash : undefined;

      const steps = 1 + Math.floor(rand() * 12);
      for (let step = 0; step < steps; step++) {
        // One generated event per step.
        const event = rand();
        if (event < 0.35) {
          now += Math.floor(rand() * 1200); // time passes
        } else if (event < 0.55) {
          watermarkIdx = Math.min(watermarks.length - 1, watermarkIdx + 1); // corpus write advances the watermark
        } else if (event < 0.7) {
          lastInvalidationAt = now; // tenant invalidation event fires
        } else if (event < 0.85) {
          sourceDown = !sourceDown; // watermark source flaps
        } else if (dataClass === "A") {
          requestedContentHash = hashes[Math.floor(rand() * hashes.length)]; // a different body is requested
        } else {
          now += Math.floor(rand() * 90);
        }

        const ctx: FreshnessContext = {
          now,
          watermark: sourceDown ? DOWN : { status: "ok", currentWatermark: watermarks[watermarkIdx] },
          requestedContentHash,
          lastInvalidationAt,
        };
        const verdict = evaluateFreshness(e, ctx, cfg);
        const model = withinBoundModel(e, ctx, cfg);
        evaluations++;
        if (verdict.fresh && !model) staleServes++;
        if (!verdict.fresh && model) missedServes++;
        expect(verdict.fresh).toBe(model);
      }
    }

    expect(evaluations).toBeGreaterThan(1000);
    expect(staleServes).toBe(0);
    expect(missedServes).toBe(0);
  });
});
