// Per-item brief (#1279): a deterministic, evidence-linked explanation of
// one Project pool item — current state, blocker, last activity, and a
// suggested next action — derived at request time from the canonical
// projection (pool item + its comment activity + the orchestration
// section). No LLM call: every claim carries an evidence link into the
// underlying data, source disagreement is stated with both sides (never
// silently resolved), and anything the projection does not know renders
// as unknown.

import type { Comment, GhState, OrchestrationSection, ProjectPoolItem, RefKey } from "./model.ts";
import { refUrl } from "./model.ts";
import { derivedProjectState, projectDisagreement, type DerivedProjectState } from "./projects.ts";

export interface BriefEvidence {
  text: string;
  url: string;
}

export interface ItemBrief {
  ref: RefKey;
  /** The brief is derived, never authored — surfaces label it as such. */
  derived: true;
  label: "Derived brief";
  state: { value: DerivedProjectState; text: string; evidence: BriefEvidence };
  blocker: {
    blocked: boolean;
    text: string;
    /** Each named blocker, linked, with its state when the pool knows it. */
    blockers: { ref: RefKey; url: string; state: GhState | "unknown" }[];
    evidence: BriefEvidence;
  };
  lastActivity: {
    /** Null when the projection records no actor — rendered "unknown". */
    actor: string | null;
    at: string | null;
    text: string;
    evidence: BriefEvidence | null;
  };
  nextAction: { text: string; evidence: BriefEvidence };
  /** Project Status vs derived state, both sides, when they disagree. */
  disagreement: { project: string; derived: DerivedProjectState; text: string; evidence: BriefEvidence } | null;
}

export interface BriefInputs {
  /** The full pool, to resolve blocker refs to their state/URL. */
  pool?: ProjectPoolItem[];
  /** Comment projection; entries for other refs are ignored. */
  comments?: Comment[];
  orchestration?: OrchestrationSection | null;
}

export function buildItemBrief(item: ProjectPoolItem, inputs: BriefInputs = {}): ItemBrief {
  const byRef = new Map((inputs.pool ?? []).map((p) => [p.ref, p]));
  const derived = derivedProjectState(item, inputs.orchestration);
  const clash = projectDisagreement(item, inputs.orchestration);
  const itemEvidence: BriefEvidence = { text: `${item.repo}#${item.number}`, url: item.issueUrl };

  // --- State -------------------------------------------------------------
  const stateText =
    derived === "Done"
      ? "Done — the linked issue/PR is closed or merged."
      : derived === "In Progress"
        ? "In Progress — open, and referenced by the orchestration plan (a tier item, a lined-up goal task, or a dependency)."
        : "Todo — open, with no orchestration reference placing it in flight.";

  // --- Blocker -----------------------------------------------------------
  // blockedBy edges are issue-only (see ProjectPoolItem). A blocker counts
  // as blocking while it is open — or while its state is unknown (absent
  // from the pool): an unknown blocker is stated as unknown, not assumed
  // resolved.
  const blockers = (item.blockedBy ?? []).map((ref) => {
    const target = byRef.get(ref);
    return {
      ref,
      url: target?.issueUrl ?? refUrl(ref, "issue"),
      state: (target?.state ?? "unknown") as GhState | "unknown",
    };
  });
  const activeBlockers = blockers.filter((b) => b.state === "open" || b.state === "unknown");
  const blocked = activeBlockers.length > 0;
  const blockerText = !blockers.length
    ? "Not blocked — no “blocked by” edges in the projection."
    : blocked
      ? `Blocked by ${activeBlockers
          .map((b) => `${b.ref}${b.state === "unknown" ? " (state unknown — not in the current pool)" : ""}`)
          .join(", ")}.`
      : `Not blocked — every recorded blocker (${blockers.map((b) => b.ref).join(", ")}) is closed or merged.`;
  const blockerEvidence: BriefEvidence = blocked
    ? { text: activeBlockers[0].ref, url: activeBlockers[0].url }
    : itemEvidence;

  // --- Last activity -------------------------------------------------------
  const activity = (inputs.comments ?? [])
    .filter((c) => c.ref === item.ref && !c.deleted)
    .sort((a, b) => (a.createdAt < b.createdAt ? -1 : a.createdAt > b.createdAt ? 1 : 0));
  const last = activity[activity.length - 1];
  const lastActivity: ItemBrief["lastActivity"] = last
    ? {
        actor: last.author,
        at: last.createdAt,
        text: `Last activity by ${last.author} at ${last.createdAt} (${last.kind}).`,
        evidence: { text: `${item.repo}#${item.number} comment`, url: last.url },
      }
    : item.updatedAt
      ? {
          actor: null,
          at: item.updatedAt,
          text: `No comment activity in the projection; last recorded item update at ${item.updatedAt} (actor unknown).`,
          evidence: itemEvidence,
        }
      : {
          actor: null,
          at: null,
          text: "Unknown — no comment activity and no update time in the projection.",
          evidence: null,
        };

  // --- Disagreement --------------------------------------------------------
  const disagreement: ItemBrief["disagreement"] = clash
    ? {
        project: clash.project,
        derived: clash.derived,
        text: `Sources disagree: the Project Status says “${clash.project}” while the rest of the dashboard derives “${clash.derived}”. Both are shown; neither is preferred.`,
        evidence: { text: `${item.repo}#${item.number} in the Project`, url: item.url },
      }
    : null;

  // --- Suggested next action (deterministic rules, first match wins) -------
  let nextAction: ItemBrief["nextAction"];
  if (derived === "Done") {
    nextAction = { text: "No action — this item is done.", evidence: itemEvidence };
  } else if (blocked) {
    nextAction = {
      text: `Resolve the blocker first: ${activeBlockers[0].ref}.`,
      evidence: { text: activeBlockers[0].ref, url: activeBlockers[0].url },
    };
  } else if (disagreement) {
    nextAction = {
      text: `Reconcile the disagreement: Project Status “${disagreement.project}” vs derived “${disagreement.derived}”.`,
      evidence: disagreement.evidence,
    };
  } else if (item.contentKind === "pr" && item.draft) {
    nextAction = { text: "Finish the draft PR and mark it ready for review.", evidence: itemEvidence };
  } else if (item.contentKind === "pr") {
    nextAction = { text: "Review the PR and merge it when checks are green.", evidence: itemEvidence };
  } else if (derived === "In Progress") {
    nextAction = {
      text: "Continue the in-flight work — the orchestration plan already references this item.",
      evidence: itemEvidence,
    };
  } else if (!activity.length) {
    nextAction = {
      text: "Triage and pick up: not blocked, not in flight, and no recorded activity yet.",
      evidence: itemEvidence,
    };
  } else {
    nextAction = { text: "Pick up or schedule: not blocked and not currently in flight.", evidence: itemEvidence };
  }

  return {
    ref: item.ref,
    derived: true,
    label: "Derived brief",
    state: { value: derived, text: stateText, evidence: itemEvidence },
    blocker: { blocked, text: blockerText, blockers, evidence: blockerEvidence },
    lastActivity,
    nextAction,
    disagreement,
  };
}

/** Briefs for a whole pool, keyed by ref — the JSON twin's shape. */
export function buildBriefs(
  pool: ProjectPoolItem[],
  inputs: Omit<BriefInputs, "pool"> = {},
): Map<RefKey, ItemBrief> {
  return new Map(pool.map((p) => [p.ref, buildItemBrief(p, { ...inputs, pool })]));
}
