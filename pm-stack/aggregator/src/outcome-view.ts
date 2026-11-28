import {
  OUTCOME_ATTRIBUTION_VERSION,
  rollupOutcomeFacts,
  type OutcomeCounts,
  type OutcomeCoverage,
  type OutcomeFact,
  type OutcomeKind,
  type OutcomeSubjectKind,
  type OutcomeUnattributedReason,
} from "./outcome-attribution.ts";

/** Outcome/rollup view for #2072 req 4, slice 4 (#10910 design spec,
 *  2026-11-18): the read side of the outcome-fact ledger. The fact
 *  store (outcome-attribution.ts) and the live consumer
 *  (outcome-consumer.ts) write and project facts; this module is the
 *  initial consumer the spec names — a rollup view over the ledger
 *  with drill-through to the exact facts behind any aggregate.
 *
 *  Honesty rules (the spec's, enforced here and in tests):
 *  - Counts come only from `rollupOutcomeFacts`, the single counting
 *    path, so the view can never disagree with a ledger rebuild.
 *  - An empty ledger and a gap-only ledger are NOT zero activity:
 *    `state` says which, and `totalByKind` is null in both. A failed
 *    sweep recorded in the ledger stays visible as `sourceGaps` even
 *    after later sweeps land facts.
 *  - Aggregates over partial-coverage facts are lower bounds: every
 *    group carries its own `partial` count so no view built on this
 *    object can render an inexact count as exact.
 *  - Every group carries the fact ids behind it, and the drill
 *    returns the facts themselves with their canonical source URLs —
 *    each count stays traceable to its exact GitHub evidence. */

export interface OutcomeGroupView {
  /** Job: the queue task id. Session: the session id. Model:
   *  `provider/model` (either side empty when the binding carried
   *  only the other), with the pair also split into the fields
   *  below — the rollup's NUL-joined internal key never leaks. */
  key: string;
  providerID: string | null;
  modelID: string | null;
  counts: OutcomeCounts;
  total: number;
  /** Facts in this group with partial coverage. */
  partial: number;
  factIds: string[];
}

export type OutcomeViewState = "ok" | "empty" | "source-gap";

export interface OutcomeView {
  attributionVersion: string;
  state: OutcomeViewState;
  /** Null unless state is "ok" — absence and gaps are not zeros. */
  totalByKind: OutcomeCounts | null;
  /** Activity facts counted (current version, source-gap excluded). */
  totalFacts: number;
  byJob: OutcomeGroupView[];
  byModel: OutcomeGroupView[];
  bySession: OutcomeGroupView[];
  unattributed: {
    total: number;
    byReason: { reason: OutcomeUnattributedReason; count: number; factIds: string[] }[];
  };
  /** Partial-coverage facts across the whole ledger. */
  partialCoverage: number;
  /** Source-gap facts recorded in the ledger (failed/absent sweeps). */
  sourceGaps: number;
}

function totalOf(counts: OutcomeCounts): number {
  return (
    counts["issue-created"] +
    counts["pr-created"] +
    counts.comment +
    counts["description-edited"] +
    counts["commit-pushed"] +
    counts.other
  );
}

interface GroupAccum {
  viewKey: string;
  providerID: string | null;
  modelID: string | null;
  factIds: string[];
  partial: number;
}

function buildGroups(
  facts: readonly OutcomeFact[],
  keyOf: (fact: OutcomeFact) => {
    rollupKey: string;
    viewKey: string;
    providerID: string | null;
    modelID: string | null;
  } | null,
  rollup: ReadonlyMap<string, OutcomeCounts>,
): OutcomeGroupView[] {
  const accums = new Map<string, GroupAccum>();
  for (const fact of facts) {
    const group = keyOf(fact);
    if (!group) continue;
    const accum = accums.get(group.rollupKey) ?? {
      viewKey: group.viewKey,
      providerID: group.providerID,
      modelID: group.modelID,
      factIds: [],
      partial: 0,
    };
    accum.factIds.push(fact.factId);
    if (fact.coverage === "partial") accum.partial += 1;
    accums.set(group.rollupKey, accum);
  }
  return [...accums.entries()]
    .map(([rollupKey, accum]) => {
      const counts = rollup.get(rollupKey);
      if (!counts) throw new Error("outcome_view_group_missing_from_rollup");
      return {
        key: accum.viewKey,
        providerID: accum.providerID,
        modelID: accum.modelID,
        counts,
        total: totalOf(counts),
        partial: accum.partial,
        factIds: accum.factIds,
      };
    })
    .sort((a, b) => b.total - a.total || a.key.localeCompare(b.key));
}

/** The model rollup keys join provider and model with a NUL separator
 *  (see rollupOutcomeFacts); the view decodes them back into the pair
 *  so consumers never parse the joined form themselves. */
function splitModelKey(key: string): { providerID: string | null; modelID: string | null } {
  const [provider, model] = key.split("\u0000");
  return { providerID: provider || null, modelID: model || null };
}

export function buildOutcomeView(facts: readonly OutcomeFact[]): OutcomeView {
  const current = facts.filter((fact) => fact.attributionVersion === OUTCOME_ATTRIBUTION_VERSION);
  const activity = current.filter((fact) => fact.method !== "source-gap");
  const sourceGaps = current.length - activity.length;

  if (activity.length === 0) {
    return {
      attributionVersion: OUTCOME_ATTRIBUTION_VERSION,
      state: sourceGaps > 0 ? "source-gap" : "empty",
      totalByKind: null,
      totalFacts: 0,
      byJob: [],
      byModel: [],
      bySession: [],
      unattributed: { total: 0, byReason: [] },
      partialCoverage: 0,
      sourceGaps,
    };
  }

  const rollup = rollupOutcomeFacts(facts);

  const reasonAccums = new Map<OutcomeUnattributedReason, string[]>();
  for (const fact of activity) {
    if (fact.method !== "unattributed" || !fact.reason) continue;
    const ids = reasonAccums.get(fact.reason) ?? [];
    ids.push(fact.factId);
    reasonAccums.set(fact.reason, ids);
  }

  return {
    attributionVersion: OUTCOME_ATTRIBUTION_VERSION,
    state: "ok",
    totalByKind: rollup.totalByKind,
    totalFacts: activity.length,
    byJob: buildGroups(
      activity,
      (fact) =>
        fact.taskId ? { rollupKey: fact.taskId, viewKey: fact.taskId, providerID: null, modelID: null } : null,
      rollup.byJob,
    ),
    byModel: buildGroups(
      activity,
      (fact) => {
        if (!fact.providerID && !fact.modelID) return null;
        const rollupKey = `${fact.providerID ?? ""}\u0000${fact.modelID ?? ""}`;
        const { providerID, modelID } = splitModelKey(rollupKey);
        return { rollupKey, viewKey: `${providerID ?? ""}/${modelID ?? ""}`, providerID, modelID };
      },
      rollup.byModel,
    ),
    bySession: buildGroups(
      activity,
      (fact) =>
        fact.sessionId ? { rollupKey: fact.sessionId, viewKey: fact.sessionId, providerID: null, modelID: null } : null,
      rollup.bySession,
    ),
    unattributed: {
      total: [...reasonAccums.values()].reduce((sum, ids) => sum + ids.length, 0),
      byReason: [...reasonAccums.entries()]
        .map(([reason, factIds]) => ({ reason, count: factIds.length, factIds }))
        .sort((a, b) => b.count - a.count || a.reason.localeCompare(b.reason)),
    },
    partialCoverage: rollup.partialCoverage,
    sourceGaps,
  };
}

export type OutcomeDrillSelector =
  | { dimension: "job"; key: string }
  | { dimension: "session"; key: string }
  | { dimension: "model"; providerID: string | null; modelID: string | null }
  | { dimension: "issue"; subjectRef: string }
  | { dimension: "unattributed"; reason: OutcomeUnattributedReason | null };

export interface OutcomeDrillRow {
  factId: string;
  eventId: string;
  outcomeKind: OutcomeKind;
  subjectKind: OutcomeSubjectKind;
  subjectRef: string | null;
  repo: string;
  actor: string | null;
  at: string | null;
  /** Canonical GitHub evidence link for this exact fact. */
  sourceUrl: string | null;
  coverage: OutcomeCoverage;
  method: string;
  reason: OutcomeUnattributedReason | null;
  taskId: string | null;
  jobId: string | null;
  sessionId: string | null;
  providerID: string | null;
  modelID: string | null;
}

function matches(fact: OutcomeFact, selector: OutcomeDrillSelector): boolean {
  switch (selector.dimension) {
    case "job":
      return fact.taskId === selector.key;
    case "session":
      return fact.sessionId === selector.key;
    case "model":
      return (fact.providerID ?? null) === selector.providerID && (fact.modelID ?? null) === selector.modelID;
    case "issue":
      return fact.subjectRef === selector.subjectRef;
    case "unattributed":
      return fact.method === "unattributed" && (selector.reason === null || fact.reason === selector.reason);
  }
}

/** The exact facts behind one aggregate, in ledger order. Only
 *  current-version activity facts can appear: source-gap facts record
 *  that a sweep failed, not activity, and a superseded version's facts
 *  are not evidence for this version's counts. */
export function drillOutcomeFacts(facts: readonly OutcomeFact[], selector: OutcomeDrillSelector): OutcomeDrillRow[] {
  return facts
    .filter(
      (fact) =>
        fact.attributionVersion === OUTCOME_ATTRIBUTION_VERSION &&
        fact.method !== "source-gap" &&
        matches(fact, selector),
    )
    .sort((a, b) => a.seq - b.seq || a.factId.localeCompare(b.factId))
    .map((fact) => ({
      factId: fact.factId,
      eventId: fact.eventId,
      outcomeKind: fact.outcomeKind,
      subjectKind: fact.subjectKind,
      subjectRef: fact.subjectRef,
      repo: fact.repo,
      actor: fact.actor,
      at: fact.at,
      sourceUrl: fact.sourceUrl,
      coverage: fact.coverage,
      method: fact.method,
      reason: fact.reason,
      taskId: fact.taskId,
      jobId: fact.jobId,
      sessionId: fact.sessionId,
      providerID: fact.providerID,
      modelID: fact.modelID,
    }));
}
