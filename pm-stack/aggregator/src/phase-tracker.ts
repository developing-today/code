// Phase tracker panel data (#9923, split from #1239): per-phase
// validation state for the id program's Phases 0-13, derived live
// from the aggregator's GitHub items — no hand-maintained values.
//
// The validation half is derivable today: the four validation audits
// under umbrella #796 map to phases by their own titles (verified
// 2026-11-14): #1075 "Phases 0-8 + platform completion acceptance
// audit", #1076 "Phase 9 portable platform", #1077 "Phase 10 worlds
// as places", #1078 "delivered Phase 11 items (claimed-done audit)".
// Phases 12 and 13 have no validation audit; the panel says so
// instead of inventing a state.
//
// The per-phase F-item PROGRESS half is derived only where a
// uniform machine-readable source exists. Phase 12: the ordered
// step list in issue #1235's body (parsed and joined by phase12.ts),
// counted by derivePhase12Progress below. Phases 11 and 13: the
// F-item mapping table in planning/roadmaps/id.md — the source of
// record created for this panel (parsed and counted by
// phase-mapping.ts) — researched on 2026-11-14: dedicated F-item
// issues for most items, the phase-11 status record for the two
// PR-delivered items (F11.6 -> PR #247, F11.14 -> PR #248), #227 for
// F11.12, and #783-#790 for F13.1-F13.8. Phases 0-10 joined the
// same table for #13944: the F-numbering mirrors their plans'
// P-step lists and each item's ref is its phase's validation audit
// (#1075/#1076/#1077), so their progress derives from live audit
// state like every other phase.
//
// Everything in this module is pure: the derivations join their
// sources against the aggregator's GhItem states.
import type { GhItem, RefKey } from "./model.ts";
import { joinPhase12Steps, parsePhase12Steps, phase12IsStale, type Phase12Snapshot } from "./phase12.ts";

export const PHASE_TRACKER_REPO = "developing-today/code";
export const PHASE_TRACKER_UMBRELLA = 796;

/** One validation audit and the phases its verdict covers. */
export interface PhaseAudit {
  issueNumber: number;
  /** Phases (0-13) this audit's verdict covers. */
  phases: readonly number[];
}

/** The audit map, from the audit issues' own titles (see header). */
export const PHASE_AUDITS: readonly PhaseAudit[] = [
  { issueNumber: 1075, phases: [0, 1, 2, 3, 4, 5, 6, 7, 8] },
  { issueNumber: 1076, phases: [9] },
  { issueNumber: 1077, phases: [10] },
  { issueNumber: 1078, phases: [11] },
];

export const PHASES = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13] as const;

/** An open issue (other than an audit itself) whose title references
 * an audit number — the fleet's follow-up linkage convention. */
export interface PhaseFollowUp {
  ref: RefKey;
  issueNumber: number;
  title: string;
  url: string;
}

export type PhaseValidationState =
  /** The covering audit is closed: the phase's validation verdict stands. */
  | "validated"
  /** The covering audit is open (filed or reopened): validation is outstanding. */
  | "validation-open"
  /** The audit could not be resolved in the current items source. */
  | "unresolved"
  /** No validation audit covers this phase. */
  | "no-audit";

/** One phase's derived validation row. */
export interface PhaseValidation {
  phase: number;
  state: PhaseValidationState;
  /** The covering audit's issue number; null when state is "no-audit". */
  auditIssue: number | null;
  auditUrl: string | null;
  /** The audit's live title when resolved (the source of the phase mapping). */
  auditTitle: string | null;
  /** The audit's live GhItem state when resolved. */
  auditState: GhItem["state"] | null;
  /** Open follow-ups referencing the covering audit. */
  openFollowUps: PhaseFollowUp[];
}

function refFor(issueNumber: number): RefKey {
  return `${PHASE_TRACKER_REPO}#${issueNumber}`;
}

function issueUrl(issueNumber: number): string {
  return `https://github.com/${PHASE_TRACKER_REPO}/issues/${issueNumber}`;
}

/** Open follow-ups for one audit: items (excluding the audits
 * themselves) whose title references the audit as "#<number>" and
 * whose live state is open. Closed references are history, not
 * outstanding validation work, so they do not surface. */
export function openFollowUpsFor(auditIssue: number, items: ReadonlyMap<RefKey, GhItem>): PhaseFollowUp[] {
  const needle = new RegExp(`#${auditIssue}\\b`);
  const auditRefs = new Set(PHASE_AUDITS.map((a) => refFor(a.issueNumber)));
  const out: PhaseFollowUp[] = [];
  for (const [ref, item] of items) {
    if (auditRefs.has(ref)) continue;
    if (item.state !== "open") continue;
    if (!needle.test(item.title)) continue;
    const m = /#(\d+)$/.exec(ref);
    out.push({
      ref,
      issueNumber: m ? Number(m[1]) : 0,
      title: item.title,
      url: item.url,
    });
  }
  return out.sort((a, b) => a.issueNumber - b.issueNumber);
}

/** Derive validation state for every phase 0-13 from live items. */
export function derivePhaseValidation(items: ReadonlyMap<RefKey, GhItem>): PhaseValidation[] {
  const byPhase = new Map<number, PhaseValidation>();
  for (const audit of PHASE_AUDITS) {
    const ref = refFor(audit.issueNumber);
    const item = items.get(ref);
    const followUps = openFollowUpsFor(audit.issueNumber, items);
    for (const phase of audit.phases) {
      byPhase.set(phase, {
        phase,
        state: !item ? "unresolved" : item.state === "open" ? "validation-open" : "validated",
        auditIssue: audit.issueNumber,
        auditUrl: item?.url ?? issueUrl(audit.issueNumber),
        auditTitle: item ? item.title : null,
        auditState: item ? item.state : null,
        openFollowUps: followUps,
      });
    }
  }
  return PHASES.map(
    (phase) =>
      byPhase.get(phase) ?? {
        phase,
        state: "no-audit" as const,
        auditIssue: null,
        auditUrl: null,
        auditTitle: null,
        auditState: null,
        openFollowUps: [],
      },
  );
}

/** Phase 12 progress, counted from the #1235 ordered step list. */
export interface Phase12Progress {
  /** Steps parsed from the current #1235 body. */
  total: number;
  /** Steps whose issue is closed in live GitHub state. */
  done: number;
  /** Steps whose issue is open in live GitHub state. */
  open: number;
  /** Steps whose issue the current items source could not resolve. */
  unresolved: number;
  /** done / total as a whole percent; null when total is 0. */
  percent: number | null;
}

/** Derive Phase 12 progress from the #1235 body snapshot joined to
 * live GhItem state. Oracle: a step is done iff its issue is CLOSED
 * in live GitHub state — completed work is recorded in GitHub
 * (closed issues, merged PRs), per the roadmap's own rule; the
 * body's checkbox is the historical record, so a checked step whose
 * issue has reopened counts as open, never as done. Returns null
 * when the feed is absent, stale, or parses to no steps: the panel
 * withholds progress rather than rendering a silently old or
 * invented percentage. */
export function derivePhase12Progress(
  snapshot: Phase12Snapshot | null,
  items: ReadonlyMap<RefKey, GhItem>,
): Phase12Progress | null {
  if (!snapshot) return null;
  if (phase12IsStale(snapshot)) return null;
  const steps = joinPhase12Steps(parsePhase12Steps(snapshot.body), items);
  if (!steps.length) return null;
  let done = 0;
  let open = 0;
  let unresolved = 0;
  for (const step of steps) {
    if (step.liveState === "closed") done++;
    else if (step.liveState === "open") open++;
    else unresolved++;
  }
  return {
    total: steps.length,
    done,
    open,
    unresolved,
    percent: Math.round((done / steps.length) * 100),
  };
}
