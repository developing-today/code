import { describe, expect, test } from "bun:test";
import type { HealthView } from "../src/health.ts";
import { sampleSloFacets } from "../src/slo-sampler.ts";

const NOW = Date.parse("2026-11-18T12:00:00Z");

function service(id: string, ok: boolean | null) {
  return { def: { id }, ok } as unknown as HealthView["services"][number];
}

function view(overrides: {
  services?: HealthView["services"];
  feedAgeMs?: number | null;
  quotaGeneratedAt?: string | null;
  orchestrationAgeMs?: number | null;
}): HealthView {
  return {
    services: overrides.services ?? [],
    feed: { ageMs: overrides.feedAgeMs ?? null },
    quotaGatherer:
      overrides.quotaGeneratedAt === null || overrides.quotaGeneratedAt === undefined
        ? null
        : { generatedAt: overrides.quotaGeneratedAt },
    jobs: { orchestration: { ageMs: overrides.orchestrationAgeMs ?? null } },
  } as unknown as HealthView;
}

const byFacet = (lines: ReturnType<typeof sampleSloFacets>) => new Map(lines.map((l) => [l.facet, l]));

describe("SLO sampler", () => {
  test("age facets are bad only past their thresholds", () => {
    const fresh = byFacet(
      sampleSloFacets(
        view({ feedAgeMs: 4 * 60_000, quotaGeneratedAt: new Date(NOW - 10 * 60_000).toISOString(), orchestrationAgeMs: 30 * 60_000 }),
        NOW,
      ),
    );
    expect(fresh.get("feed-age")).toMatchObject({ bad: false, value: 4 * 60_000 });
    expect(fresh.get("quota-age")).toMatchObject({ bad: false, value: 10 * 60_000 });
    expect(fresh.get("orchestration-age")).toMatchObject({ bad: false });

    const stale = byFacet(
      sampleSloFacets(
        view({ feedAgeMs: 11 * 60_000, quotaGeneratedAt: new Date(NOW - 80 * 60_000).toISOString(), orchestrationAgeMs: 95 * 60_000 }),
        NOW,
      ),
    );
    expect(stale.get("feed-age")?.bad).toBe(true);
    expect(stale.get("quota-age")?.bad).toBe(true);
    expect(stale.get("orchestration-age")?.bad).toBe(true);
  });

  test("missing age sources record unknown, never good", () => {
    const missing = byFacet(sampleSloFacets(view({}), NOW));
    expect(missing.get("feed-age")?.bad).toBeNull();
    expect(missing.get("quota-age")?.bad).toBeNull();
    expect(missing.get("orchestration-age")?.bad).toBeNull();
  });

  test("up facets follow the probed services and are unknown when unprobed", () => {
    const sampled = byFacet(
      sampleSloFacets(
        view({
          services: [
            service("aggregator", true),
            service("projector", false),
            service("pad", true),
            service("grist", true),
          ],
        }),
        NOW,
      ),
    );
    expect(sampled.get("aggregator-up")?.bad).toBe(false);
    expect(sampled.get("projector-up")?.bad).toBe(true);
    expect(sampled.get("pad-grist-up")?.bad).toBe(false);
    expect(sampled.get("intake-up")?.bad).toBeNull();
    expect(sampled.get("tunnel-up")?.bad).toBeNull();
  });

  test("emits one line per facet at the given minute", () => {
    const lines = sampleSloFacets(view({}), NOW);
    expect(lines).toHaveLength(8);
    expect(new Set(lines.map((l) => l.facet)).size).toBe(8);
    expect(lines.every((l) => l.atMs === NOW)).toBe(true);
  });
});
