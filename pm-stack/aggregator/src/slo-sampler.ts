import type { HealthView, ServiceHealth } from "./health.ts";
import type { SloJournalLine } from "./slo-journal.ts";

const MINUTE = 60_000;

/** Per-facet bad thresholds, mirroring the SLO targets in slo.ts and the
 * design note. Age facets are bad past their threshold; up facets are bad
 * when a probed service answers not-ok. A facet with no probe or no age
 * source records bad: null so it is excluded, never called good. */
const FEED_BAD_AGE_MS = 10 * MINUTE;
const QUOTA_BAD_AGE_MS = 75 * MINUTE;
const ORCHESTRATION_BAD_AGE_MS = 90 * MINUTE;

function serviceOk(view: HealthView, ids: readonly string[]): boolean | null {
  const rows: ServiceHealth[] = ids
    .map((id) => view.services.find((s) => s.def.id === id))
    .filter((s): s is ServiceHealth => s !== undefined);
  if (rows.length !== ids.length) return null;
  if (rows.some((s) => s.ok === false)) return false;
  if (rows.every((s) => s.ok === true)) return true;
  return null;
}

function upSample(ids: readonly string[], view: HealthView): { bad: boolean | null; value: null } {
  const ok = serviceOk(view, ids);
  return { bad: ok === null ? null : !ok, value: null };
}

/** Sample every SLO facet once from an assembled health view (#9988). Pure:
 * the caller supplies the view and the minute timestamp. */
export function sampleSloFacets(view: HealthView, atMs: number): SloJournalLine[] {
  const quotaAgeMs = view.quotaGatherer ? Math.max(0, atMs - Date.parse(view.quotaGatherer.generatedAt)) : null;
  const orchestrationAgeMs = view.jobs.orchestration.ageMs ?? null;
  const feedAgeMs = view.feed.ageMs;
  const facets: SloJournalLine[] = [
    {
      facet: "feed-age",
      atMs,
      value: feedAgeMs,
      bad: feedAgeMs === null ? null : feedAgeMs > FEED_BAD_AGE_MS,
    },
    {
      facet: "quota-age",
      atMs,
      value: quotaAgeMs,
      bad: quotaAgeMs === null ? null : quotaAgeMs > QUOTA_BAD_AGE_MS,
    },
    {
      facet: "orchestration-age",
      atMs,
      value: orchestrationAgeMs,
      bad: orchestrationAgeMs === null ? null : orchestrationAgeMs > ORCHESTRATION_BAD_AGE_MS,
    },
    { facet: "aggregator-up", atMs, ...upSample(["aggregator"], view) },
    { facet: "intake-up", atMs, ...upSample(["intake"], view) },
    { facet: "projector-up", atMs, ...upSample(["projector"], view) },
    { facet: "pad-grist-up", atMs, ...upSample(["pad", "grist"], view) },
    { facet: "tunnel-up", atMs, ...upSample(["tunnel"], view) },
  ];
  return facets;
}
