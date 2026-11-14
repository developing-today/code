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
// The per-phase F-item PROGRESS half is deliberately not derived
// here: no uniform machine-readable F-item mapping exists for
// Phases 0-13 (archaeology on #9923, 2026-11-14), and inventing one
// in this module would be exactly the hand-maintained input the
// panel forbids. Progress joins when a phase-mapping source of
// record exists (proposed: a phases table in planning/roadmaps/id.md,
// parsed like the repo-roadmap source); until then those phases
// render validation state only.
//
// Everything in this module is pure: the derivation joins the
// audit map against the aggregator's GhItem states.
import type { GhItem, RefKey } from "./model.ts";

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
export function openFollowUpsFor(
  auditIssue: number,
  items: ReadonlyMap<RefKey, GhItem>,
): PhaseFollowUp[] {
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
export function derivePhaseValidation(
  items: ReadonlyMap<RefKey, GhItem>,
): PhaseValidation[] {
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
