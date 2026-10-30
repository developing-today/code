// Model-choice guidance for the quota view (#851): which pools have
// headroom for a new session, which are about to reset with unused
// headroom (burn-it-before-reset), and which are too close to their
// limit to start anything on. Pure derivation over QuotaPool rows —
// used by the fleet-feed collector; the aggregator renders the result.
import { poolState, windowPercent } from "./derive.ts";
import type { QuotaPool, QuotaView } from "./model.ts";

/** A reset inside this window counts as "about to reset". */
export const RESET_SOON_MS = 24 * 3_600_000;

function maxPercent(p: QuotaPool): number {
  if (p.windows.length === 0) return 0;
  return Math.max(...p.windows.map(windowPercent));
}

function soonestReset(p: QuotaPool): number | null {
  const times = p.windows.map((w) => (w.resetsAt ? Date.parse(w.resetsAt) : NaN)).filter((t) => Number.isFinite(t));
  return times.length ? Math.min(...times) : null;
}

export function computeChoices(pools: QuotaPool[], now = Date.now()): QuotaView["choices"] {
  const hasHeadroom: QuotaPool[] = [];
  const burnBeforeReset: QuotaPool[] = [];
  const doNotStart: QuotaPool[] = [];

  for (const p of pools) {
    if (p.windows.length === 0) continue; // no current read — no guidance
    const state = poolState(p);
    if (state === "walled" || state === "near") {
      doNotStart.push(p);
      continue;
    }
    hasHeadroom.push(p);
    const reset = soonestReset(p);
    if (reset !== null && reset - now <= RESET_SOON_MS && reset > now) {
      burnBeforeReset.push(p);
    }
  }

  // Best headroom first: lowest peak-window usage first.
  hasHeadroom.sort((a, b) => maxPercent(a) - maxPercent(b));
  // Soonest reset first.
  burnBeforeReset.sort((a, b) => (soonestReset(a) ?? 0) - (soonestReset(b) ?? 0));

  return {
    hasHeadroom: hasHeadroom.map((p) => p.id),
    burnBeforeReset: burnBeforeReset.map((p) => p.id),
    doNotStart: doNotStart.map((p) => p.id),
  };
}
