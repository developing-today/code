import { appendFile, mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import { githubHotEventDelivery, type GithubHotEvent } from "./activity.ts";

/** Outcome attribution for #2072 req 4, slice 4 (#10910 design spec,
 *  2026-11-18). Spend attribution (spend-attribution.ts) answers "what
 *  measurable usage was attributed where"; this module answers "what
 *  work product happened": one append-only outcome fact per normalized
 *  #5204 hot-list event, with counts projected by job / model / session.
 *
 *  Increment when the rules that produce outcome facts change. Facts
 *  are immutable by version: a correction is a new fact under a new
 *  version, never a rewrite, and rollups read one version at a time. */
export const OUTCOME_ATTRIBUTION_VERSION = "v1";

export type OutcomeKind = "issue-created" | "pr-created" | "comment" | "description-edited" | "commit-pushed" | "other";

export type OutcomeSubjectKind =
  | "issue"
  | "pr"
  | "comment"
  | "review"
  | "review-comment"
  | "commit"
  | "push"
  | "unknown";

export type OutcomeCoverage = "complete" | "partial";
export type OutcomeAttributionMethod = "queue-binding" | "unattributed" | "source-gap";
export type OutcomeUnattributedReason = "no-binding" | "source-gap";

/** All six kinds, zero-filled: an empty successful sweep is a record of
 *  zeros, which stays distinguishable from a failed sweep (null totals). */
export interface OutcomeCounts {
  "issue-created": number;
  "pr-created": number;
  comment: number;
  "description-edited": number;
  "commit-pushed": number;
  other: number;
}

function zeroCounts(): OutcomeCounts {
  return {
    "issue-created": 0,
    "pr-created": 0,
    comment: 0,
    "description-edited": 0,
    "commit-pushed": 0,
    other: 0,
  };
}

/** An authoritative join between one hot-list event and the delegated
 *  work that produced it, supplied by the caller from a binding record
 *  (queue claim, session binding, schedule record). The projector never
 *  invents one: a GitHub actor, a branch name, or time proximity is not
 *  a binding (#10910 spec). `taskId` is the queue task — the job key;
 *  `jobId` preserves a schedule/job id separately when a source
 *  supplies one; the two are never conflated. */
export interface OutcomeBinding {
  eventId: string;
  taskId?: string | null;
  jobId?: string | null;
  sessionId?: string | null;
  rootSessionId?: string | null;
  parentSessionId?: string | null;
  providerID?: string | null;
  modelID?: string | null;
  /** How the binding is authoritative, e.g. "queue-binding". */
  method: string;
}

export interface OutcomeFact {
  factId: string;
  attributionVersion: string;
  /** The original #5204 hot-list event id; every aggregate drills back
   *  to the exact GitHub evidence through it and `sourceUrl`. */
  eventId: string;
  seq: number;
  repo: string;
  /** Raw hot-list event name ("issues", "push", ...). */
  event: string;
  action: string | null;
  outcomeKind: OutcomeKind;
  subjectKind: OutcomeSubjectKind;
  /** `repo#number` for issue/PR subjects, the branch ref for pushes. */
  subjectRef: string | null;
  actor: string | null;
  sourceUrl: string | null;
  /** Normalized source timestamp (source time, received time as
   *  fallback); null only on the synthetic source-gap fact. */
  at: string | null;
  sourceTimestamp: string | null;
  receivedAt: string | null;
  taskId: string | null;
  jobId: string | null;
  sessionId: string | null;
  rootSessionId: string | null;
  parentSessionId: string | null;
  providerID: string | null;
  modelID: string | null;
  method: OutcomeAttributionMethod | string;
  reason: OutcomeUnattributedReason | null;
  /** "partial" when the source row cannot prove the exact count it
   *  stands for (the zero-commit push fallback row): the fact counts
   *  once, but no view may render the aggregate as exact. */
  coverage: OutcomeCoverage;
}

export interface OutcomeAttributionInput {
  /** A failed or missing hot list is not equivalent to an empty one. */
  sourceStatus: "ok" | "missing" | "error";
  events: readonly GithubHotEvent[];
  bindings?: readonly OutcomeBinding[];
}

export interface OutcomeRollup {
  /** Null only when built from a failed/absent source projection. */
  totalByKind: OutcomeCounts | null;
  byJob: Map<string, OutcomeCounts>;
  byModel: Map<string, OutcomeCounts>;
  bySession: Map<string, OutcomeCounts>;
  unattributedByReason: Map<OutcomeUnattributedReason, number>;
  /** Facts whose coverage is partial; aggregates over them are lower
   *  bounds, not exact counts. */
  partialCoverage: number;
}

export interface OutcomeAttributionProjection extends OutcomeRollup {
  attributionVersion: string;
  sourceStatus: "ok" | "missing" | "error";
  facts: OutcomeFact[];
}

function classify(event: GithubHotEvent): {
  outcomeKind: OutcomeKind;
  subjectKind: OutcomeSubjectKind;
  coverage: OutcomeCoverage;
} {
  const action = event.action?.trim() ?? "";
  switch (event.event.trim()) {
    case "issues":
      if (action === "opened") return { outcomeKind: "issue-created", subjectKind: "issue", coverage: "complete" };
      if (action === "edited") return { outcomeKind: "description-edited", subjectKind: "issue", coverage: "complete" };
      return { outcomeKind: "other", subjectKind: "issue", coverage: "complete" };
    case "pull_request":
      if (action === "opened") return { outcomeKind: "pr-created", subjectKind: "pr", coverage: "complete" };
      if (action === "edited") return { outcomeKind: "description-edited", subjectKind: "pr", coverage: "complete" };
      return { outcomeKind: "other", subjectKind: "pr", coverage: "complete" };
    case "issue_comment":
      return action === "created"
        ? { outcomeKind: "comment", subjectKind: "comment", coverage: "complete" }
        : { outcomeKind: "other", subjectKind: "comment", coverage: "complete" };
    case "pull_request_review":
      return action === "submitted"
        ? { outcomeKind: "comment", subjectKind: "review", coverage: "complete" }
        : { outcomeKind: "other", subjectKind: "review", coverage: "complete" };
    case "pull_request_review_comment":
      return action === "created"
        ? { outcomeKind: "comment", subjectKind: "review-comment", coverage: "complete" }
        : { outcomeKind: "other", subjectKind: "review-comment", coverage: "complete" };
    case "push": {
      // The projector emits one hot row per commit (`commit:<sha>` in
      // the stable event id) plus, for a push whose commit list it
      // could not expose, a single fallback row (`push:<after-sha>`).
      // Only the per-commit rows prove an exact commit count.
      const perCommit = event.event_id.split(":").includes("commit");
      return perCommit
        ? { outcomeKind: "commit-pushed", subjectKind: "commit", coverage: "complete" }
        : { outcomeKind: "commit-pushed", subjectKind: "push", coverage: "partial" };
    }
    default:
      return { outcomeKind: "other", subjectKind: "unknown", coverage: "complete" };
  }
}

function sourceGapFact(): OutcomeFact {
  return {
    factId: `outcome:${OUTCOME_ATTRIBUTION_VERSION}:source-gap`,
    attributionVersion: OUTCOME_ATTRIBUTION_VERSION,
    eventId: "source-gap",
    seq: 0,
    repo: "",
    event: "",
    action: null,
    outcomeKind: "other",
    subjectKind: "unknown",
    subjectRef: null,
    actor: null,
    sourceUrl: null,
    at: null,
    sourceTimestamp: null,
    receivedAt: null,
    taskId: null,
    jobId: null,
    sessionId: null,
    rootSessionId: null,
    parentSessionId: null,
    providerID: null,
    modelID: null,
    method: "source-gap",
    reason: "source-gap",
    coverage: "complete",
  };
}

function addToGroup(groups: Map<string, OutcomeCounts>, key: string, kind: OutcomeKind): void {
  const counts = groups.get(key) ?? zeroCounts();
  counts[kind] += 1;
  groups.set(key, counts);
}

/** Fold facts into counts. Source-gap facts are excluded: they record
 *  that a sweep failed, not activity. Only facts at the current
 *  attribution version are counted, so a re-projection under a new
 *  version supersedes rather than double-counts. This is the single
 *  counting path — live projections and ledger rebuilds both use it,
 *  which is what makes the counts rebuildable by construction. */
export function rollupOutcomeFacts(facts: readonly OutcomeFact[]): OutcomeRollup {
  const totalByKind = zeroCounts();
  const byJob = new Map<string, OutcomeCounts>();
  const byModel = new Map<string, OutcomeCounts>();
  const bySession = new Map<string, OutcomeCounts>();
  const unattributedByReason = new Map<OutcomeUnattributedReason, number>();
  let partialCoverage = 0;

  for (const fact of facts) {
    if (fact.attributionVersion !== OUTCOME_ATTRIBUTION_VERSION) continue;
    if (fact.method === "source-gap") continue;
    totalByKind[fact.outcomeKind] += 1;
    if (fact.coverage === "partial") partialCoverage += 1;
    if (fact.taskId) addToGroup(byJob, fact.taskId, fact.outcomeKind);
    if (fact.providerID || fact.modelID)
      addToGroup(byModel, `${fact.providerID ?? ""}\u0000${fact.modelID ?? ""}`, fact.outcomeKind);
    if (fact.sessionId) addToGroup(bySession, fact.sessionId, fact.outcomeKind);
    if (fact.method === "unattributed" && fact.reason)
      unattributedByReason.set(fact.reason, (unattributedByReason.get(fact.reason) ?? 0) + 1);
  }

  return { totalByKind, byJob, byModel, bySession, unattributedByReason, partialCoverage };
}

/** Project normalized #5204 hot-list rows into append-only outcome
 *  facts. Deterministic: facts are sorted by (seq, event id), replayed
 *  event ids dedupe to their first occurrence, and identities attach
 *  only through caller-supplied authoritative bindings. Malformed rows
 *  throw the activity contract's `invalid_hot_event_*` errors rather
 *  than minting facts from them. */
export function projectOutcomeAttribution(input: OutcomeAttributionInput): OutcomeAttributionProjection {
  if (input.sourceStatus !== "ok") {
    const facts = [sourceGapFact()];
    return {
      attributionVersion: OUTCOME_ATTRIBUTION_VERSION,
      sourceStatus: input.sourceStatus,
      facts,
      totalByKind: null,
      byJob: new Map(),
      byModel: new Map(),
      bySession: new Map(),
      unattributedByReason: new Map(),
      partialCoverage: 0,
    };
  }

  const bindingsByEvent = new Map((input.bindings ?? []).map((binding) => [binding.eventId, binding] as const));
  const sorted = [...input.events].sort((a, b) => a.seq - b.seq || a.event_id.localeCompare(b.event_id));
  const seen = new Set<string>();
  const facts: OutcomeFact[] = [];

  for (const event of sorted) {
    const eventId = event.event_id.trim();
    if (seen.has(eventId)) continue;
    seen.add(eventId);
    // Reuses the activity contract's validation (id / repo / kind /
    // timestamp) and its canonical source-link derivation, so an
    // outcome fact and the activity stream agree on the evidence.
    const delivery = githubHotEventDelivery(event);
    const { outcomeKind, subjectKind, coverage } = classify(event);
    const binding = bindingsByEvent.get(eventId);
    facts.push({
      factId: `outcome:${OUTCOME_ATTRIBUTION_VERSION}:${eventId}`,
      attributionVersion: OUTCOME_ATTRIBUTION_VERSION,
      eventId,
      seq: event.seq,
      repo: event.repo.trim(),
      event: event.event.trim(),
      action: event.action?.trim() || null,
      outcomeKind,
      subjectKind,
      subjectRef: event.ref?.trim() || null,
      actor: event.actor?.trim() || null,
      sourceUrl: delivery.link ?? null,
      at: delivery.at,
      sourceTimestamp: event.source_timestamp,
      receivedAt: event.received_at,
      taskId: binding?.taskId ?? null,
      jobId: binding?.jobId ?? null,
      sessionId: binding?.sessionId ?? null,
      rootSessionId: binding?.rootSessionId ?? null,
      parentSessionId: binding?.parentSessionId ?? null,
      providerID: binding?.providerID ?? null,
      modelID: binding?.modelID ?? null,
      method: binding ? binding.method : "unattributed",
      reason: binding ? null : "no-binding",
      coverage,
    });
  }

  return {
    attributionVersion: OUTCOME_ATTRIBUTION_VERSION,
    sourceStatus: input.sourceStatus,
    facts,
    ...rollupOutcomeFacts(facts),
  };
}

export interface OutcomeSweepResult {
  projection: OutcomeAttributionProjection;
  /** Facts newly appended by this sweep (0 on a pure replay). */
  appended: number;
  /** Highest hot-list seq durably represented in the ledger. Advances
   *  only after the sweep's facts are durably written, and never on a
   *  failed sweep. */
  cursor: number;
}

/** Append-only JSONL ledger for outcome facts, following the
 *  aggregator's file-store discipline: stable fact-id dedupe, and the
 *  cursor state written (tmp + rename) only after the facts it covers
 *  are on disk. A missing ledger reads as empty; a corrupt line is an
 *  explicit `outcome_fact_store_corrupt` error, never a silent skip —
 *  a gap in the evidence must not render as zero activity. */
export class OutcomeFactStore {
  private readonly stateFile: string;

  constructor(private readonly file: string) {
    this.stateFile = `${file}.state.json`;
  }

  async readFacts(): Promise<OutcomeFact[]> {
    let text: string;
    try {
      text = await readFile(this.file, "utf8");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return [];
      throw error;
    }
    const facts: OutcomeFact[] = [];
    for (const line of text.split("\n")) {
      if (!line.trim()) continue;
      try {
        facts.push(JSON.parse(line) as OutcomeFact);
      } catch {
        throw new Error("outcome_fact_store_corrupt");
      }
    }
    return facts;
  }

  async cursor(): Promise<number> {
    let text: string;
    try {
      text = await readFile(this.stateFile, "utf8");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return 0;
      throw error;
    }
    try {
      const state = JSON.parse(text) as { cursor?: unknown };
      return typeof state.cursor === "number" && Number.isSafeInteger(state.cursor) && state.cursor >= 0
        ? state.cursor
        : 0;
    } catch {
      throw new Error("outcome_fact_store_corrupt");
    }
  }

  /** Project one sweep and durably append its new facts. Replay-safe:
   *  fact-id dedupe is the correctness mechanism, so re-running a
   *  sweep from an older checkpoint appends nothing twice. */
  async applySweep(input: OutcomeAttributionInput): Promise<OutcomeSweepResult> {
    const projection = projectOutcomeAttribution(input);
    const existing = new Set((await this.readFacts()).map((fact) => fact.factId));
    const fresh = projection.facts.filter((fact) => !existing.has(fact.factId));
    const cursor = await this.cursor();
    if (fresh.length > 0) {
      await mkdir(dirname(this.file), { recursive: true });
      await appendFile(this.file, fresh.map((fact) => `${JSON.stringify(fact)}\n`).join(""), "utf8");
    }
    if (input.sourceStatus !== "ok") return { projection, appended: fresh.length, cursor };
    const maxSeq = input.events.reduce((max, event) => Math.max(max, event.seq), cursor);
    if (maxSeq !== cursor) {
      await mkdir(dirname(this.stateFile), { recursive: true });
      const tmp = `${this.stateFile}.tmp`;
      await writeFile(tmp, `${JSON.stringify({ cursor: maxSeq })}\n`, "utf8");
      await rename(tmp, this.stateFile);
    }
    return { projection, appended: fresh.length, cursor: maxSeq };
  }
}
