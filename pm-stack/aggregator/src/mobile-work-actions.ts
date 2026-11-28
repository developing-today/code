// The MOB-06 safe status/note/comment/priority contract (#9473,
// pm-stack design package
// `2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration`, component
// 9201 §§3.2–3.6, slice MOB-06).
//
// Why this module exists: the mobile controls inventory (items 4–8: add a
// private note, add/reply to a public comment, change urgency, set/clear
// `do-now`, request a permitted Ready/Parked/Blocked state) must each
// resolve to exactly one policy result and exactly one expected
// source-of-truth change. One result and one change per verb is what makes
// the cross-surface golden corpus hold: no verb can fan out to two
// writers, so no sync loop is expressible here.
//
// Design defaults pinned by this contract:
// - a note is private unless the public mirror was explicitly selected AND
//   previewed; an unpreviewed mirror is rejected before any effect;
// - urgency never carries a locally computed score: an allowed
//   `urgency.change` records a canonical-model re-run (`reruns9200`,
//   the #9200 seam) and protected labels (`expedite`, `next-up`, the
//   ratified #9452 values) require the owner;
// - `do-now` is a one-shot directive and never modifies urgency; it is
//   refused on blocked items;
// - status requests are refused when a hard blocker is present or the
//   Project status is unsupported — never silently mapped;
// - a stale expected work version ends in `conflict`, an unreadable
//   authoritative version in `uncertain`, never a silent apply.
//
// Boundaries: pure module, no I/O, no clock, no downstream client. The
// caller supplies the work-version reader; the fake orchestrator/Fleet
// owners live in the test. Production adoption, the Fleet-side
// operations seam (#9178/#9179 gates) and live acceptance stay with
// their own slices; this file is the executable contract the design
// asks for. The #9200 score mapping (PRI-01) is named only as the
// re-run seam — this module computes no score.

/** Controls inventory items 4–8, in design order. */
export const MOBILE_WORK_VERBS = [
  "note.private",
  "comment.public",
  "urgency.change",
  "do-now.set",
  "do-now.clear",
  "status.request",
] as const;

export type MobileWorkVerb = (typeof MOBILE_WORK_VERBS)[number];

/** The single policy outcome. Exactly one per evaluation. */
export type MobileWorkPolicy = "allowed" | "rejected" | "conflict" | "uncertain";

/** Stable reason codes. `ok.*` allow, `rejected.*` refuse before any
 *  effect, `uncertain.*` report an unreadable authority. */
export const MOBILE_WORK_REASONS = [
  "ok.private_note_default",
  "ok.public_mirror_previewed",
  "ok.public_comment_confirmed",
  "ok.urgency_rerun_9200",
  "ok.do_now_set",
  "ok.do_now_clear",
  "ok.status_permitted",
  "rejected.public_mirror_not_previewed",
  "rejected.public_confirmation_required",
  "rejected.noncanonical_priority_labels",
  "rejected.protected_expedite_edit",
  "rejected.blocked_do_now",
  "rejected.hard_blocker_present",
  "rejected.unsupported_project_status",
  "conflict.stale_expected_version",
  "uncertain.unreadable_authority",
] as const;

export type MobileWorkReason = (typeof MOBILE_WORK_REASONS)[number];

/**
 * The one owner surface an allowed verb writes. One surface per verb is
 * the no-sync-loop invariant: Project writes stay with the orchestrator
 * single writer, queue facts with the broker, and this contract names
 * which Fleet-owned surface the request targets without writing it.
 */
export const MOBILE_WORK_SURFACES = [
  "fleet.private-journal",
  "fleet.public-comment",
  "fleet.urgency-rerun-9200",
  "fleet.directive",
  "fleet.status-request",
] as const;

export type MobileWorkSurface = (typeof MOBILE_WORK_SURFACES)[number];

export type MobileWorkLifecycle =
  | "inbox"
  | "triage"
  | "backlog"
  | "ready"
  | "in-progress"
  | "review"
  | "blocked"
  | "parked"
  | "done";

export interface MobileWorkSnapshot {
  objectRef: string;
  /** The work version the client believes it is acting on. */
  expectedWorkVersion: string;
  lifecycle: MobileWorkLifecycle;
  /** A native hard blocker (blockedBy) the request must not bypass. */
  hardBlocker: boolean;
  projectStatus: string;
  /** False when the Project status has no mapping in the projection. */
  projectStatusSupported: boolean;
  /** False when the item's priority labels contradict each other or the canon. */
  priorityLabelsCanonical: boolean;
}

export interface MobileWorkArgs {
  body?: string;
  /** Public mirror explicitly selected by the author. */
  publicMirrorSelected?: boolean;
  /** …and previewed, so the author saw exactly what would go public. */
  publicMirrorPreviewed?: boolean;
  /** Explicit public intent for a public comment. */
  publicConfirmed?: boolean;
  /** Urgency label being requested, e.g. "high". Protected values
   *  ("expedite", "next-up") require the owner. */
  urgencyTarget?: string;
  /** Whether the actor is the owner (Drew). Client-supplied context
   *  only; the server re-derives authority at apply time. */
  actorIsDrew?: boolean;
  /** Permitted Ready/Parked/Blocked target for status.request. */
  statusTarget?: "ready" | "parked" | "blocked";
}

/** Server-side collaborators. Injected so the contract has no ambient authority. */
export interface MobileWorkAuthority {
  /** Current authoritative work version for the object, or null when unreadable. */
  readWorkVersion(objectRef: string): string | null;
}

export interface MobileWorkChange {
  /** The single owner surface this verb targets. */
  surface: MobileWorkSurface;
  detail: string;
  /** True only for urgency.change: priority is recomputed by the
   *  canonical #9200 model, never locally. */
  reruns9200: boolean;
  /** True only for do-now verbs: urgency labels are untouched. */
  urgencyUnchanged: boolean;
  /** True only when an explicitly previewed public mirror rides along. */
  mirroredPublic: boolean;
}

export interface MobileWorkDecision {
  policy: MobileWorkPolicy;
  reasonCode: MobileWorkReason;
  /** Present exactly when policy is "allowed". */
  change: MobileWorkChange | null;
}

/** Labels whose edit is owner-only, per the ratified #9452 values. */
const PROTECTED_URGENCY_TARGETS = new Set(["expedite", "next-up"]);

function noChange(policy: MobileWorkPolicy, reasonCode: MobileWorkReason): MobileWorkDecision {
  return { policy, reasonCode, change: null };
}

/**
 * Decide one mobile verb against the snapshot and the fake authority.
 * Global gates run first, in design §3.5 order: an unreadable
 * authoritative version is `uncertain`, a moved version is `conflict`.
 * Verb policy runs only on a fresh, readable version.
 */
export function decideMobileWorkAction(
  verb: MobileWorkVerb,
  args: MobileWorkArgs,
  snapshot: MobileWorkSnapshot,
  authority: MobileWorkAuthority,
): MobileWorkDecision {
  const current = authority.readWorkVersion(snapshot.objectRef);
  if (current === null) {
    return noChange("uncertain", "uncertain.unreadable_authority");
  }
  if (current !== snapshot.expectedWorkVersion) {
    return { policy: "conflict", reasonCode: "conflict.stale_expected_version", change: null };
  }

  switch (verb) {
    case "note.private": {
      if (args.publicMirrorSelected && !args.publicMirrorPreviewed) {
        return noChange("rejected", "rejected.public_mirror_not_previewed");
      }
      const mirrored = Boolean(args.publicMirrorSelected && args.publicMirrorPreviewed);
      return {
        policy: "allowed",
        reasonCode: mirrored ? "ok.public_mirror_previewed" : "ok.private_note_default",
        change: {
          surface: "fleet.private-journal",
          detail: args.body ?? "",
          reruns9200: false,
          urgencyUnchanged: false,
          mirroredPublic: mirrored,
        },
      };
    }
    case "comment.public": {
      if (!args.publicConfirmed) {
        return noChange("rejected", "rejected.public_confirmation_required");
      }
      return {
        policy: "allowed",
        reasonCode: "ok.public_comment_confirmed",
        change: {
          surface: "fleet.public-comment",
          detail: args.body ?? "",
          reruns9200: false,
          urgencyUnchanged: false,
          mirroredPublic: false,
        },
      };
    }
    case "urgency.change": {
      if (!snapshot.priorityLabelsCanonical) {
        return noChange("rejected", "rejected.noncanonical_priority_labels");
      }
      if (
        args.urgencyTarget !== undefined &&
        PROTECTED_URGENCY_TARGETS.has(args.urgencyTarget) &&
        !args.actorIsDrew
      ) {
        return noChange("rejected", "rejected.protected_expedite_edit");
      }
      return {
        policy: "allowed",
        reasonCode: "ok.urgency_rerun_9200",
        change: {
          surface: "fleet.urgency-rerun-9200",
          detail: args.urgencyTarget ?? "",
          reruns9200: true,
          urgencyUnchanged: false,
          mirroredPublic: false,
        },
      };
    }
    case "do-now.set": {
      if (snapshot.lifecycle === "blocked") {
        return noChange("rejected", "rejected.blocked_do_now");
      }
      return {
        policy: "allowed",
        reasonCode: "ok.do_now_set",
        change: {
          surface: "fleet.directive",
          detail: "do-now set (one-shot; urgency untouched)",
          reruns9200: false,
          urgencyUnchanged: true,
          mirroredPublic: false,
        },
      };
    }
    case "do-now.clear": {
      return {
        policy: "allowed",
        reasonCode: "ok.do_now_clear",
        change: {
          surface: "fleet.directive",
          detail: "do-now cleared (urgency untouched)",
          reruns9200: false,
          urgencyUnchanged: true,
          mirroredPublic: false,
        },
      };
    }
    case "status.request": {
      if (snapshot.hardBlocker) {
        return noChange("rejected", "rejected.hard_blocker_present");
      }
      if (!snapshot.projectStatusSupported) {
        return noChange("rejected", "rejected.unsupported_project_status");
      }
      return {
        policy: "allowed",
        reasonCode: "ok.status_permitted",
        change: {
          surface: "fleet.status-request",
          detail: args.statusTarget ?? "",
          reruns9200: false,
          urgencyUnchanged: false,
          mirroredPublic: false,
        },
      };
    }
  }
}
