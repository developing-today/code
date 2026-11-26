// MOB-06 — safe status, note, comment and priority controls (#9473;
// design package `2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration`,
// component 9201 §3.3 items 4–8, slice MOB-06; PRI-06 merged into it).
//
// This is the status/priority policy adapter the slice names. It sits in
// front of the MOB-05 envelope (src/work-action-contract.ts): a mobile or
// browser control asks `planMobileAction` what one verb would do, gets
// back exactly one policy result, and only an `allow` result becomes a
// MOB-05 request via `toWorkActionRequest`. Preview/accept/apply,
// operation keys, the expected work version (a conflicting revision ends
// in `conflict`) and re-authorization stay with MOB-05.
//
// What the adapter decides, stated once:
//
// * Item 4, private note: private by default. A public mirror happens
//   only when the request selects it AND says the mirror was previewed;
//   otherwise the result is a denial, never a quiet public comment.
// * Item 5, public comment or reply: public by definition, so it needs
//   an explicit confirmation (C-R083: confirmations for consequential
//   actions).
// * Item 6, urgency: the value must be a member of the queue profile
//   `fleet.queue@1` (the PRI-01 golden, #7030's `expedite = 15` and
//   `next-up = 25` included, never snapped). Priority labels sent with
//   the request must be canonical and agree with the value. Every write
//   crosses the protected-owner gate exactly as PRI-01's
//   `check_priority_write` does, and every allowed write asks the #9200
//   mapping to re-run; the adapter never writes a label itself.
// * Item 7, `do-now`: a one-shot directive, separate from urgency. It
//   never changes the score and cannot bypass blocked, parked,
//   `manual_only` or terminal state.
// * Item 8, Ready/Parked/Blocked: Ready is refused while a hard blocker
//   is open; Parked needs a reason and a checkpoint (typed `work.park`);
//   Blocked needs a reason. A Project #4 Status request is a separate
//   verb, limited to the Project vocabulary in status-taxonomy.ts, routed
//   to the orchestrator (the Project's single writer), and `Done` is
//   never requestable because visible Done needs a verified receipt.
//
// Boundaries: pure module, no I/O, no clock. The caller supplies the item
// facts and the owner policy it read under the same work version. No new
// store, queue, Project writer or ranking engine is introduced here.

import type { WorkActionRequest } from "./work-action-contract.ts";

/** The PRI-01 queue profile golden (`queue_v1_golden` in priority.rs). */
export const QUEUE_PROFILE_ID = "fleet.queue";
export const QUEUE_PROFILE_VERSION = "1";
export const QUEUE_V1_GOLDEN: ReadonlyMap<number, string> = new Map([
  [10, "critical"],
  [15, "expedite"],
  [20, "high"],
  [25, "next-up"],
  [30, "scale-30"],
  [40, "normal"],
  [50, "scale-50"],
  [70, "low"],
  [90, "scale-90"],
]);

/** Canonical priority mirror labels, as the repo spells them: the
 *  `priority/<name>` labels for the four named scale values
 *  (pm-stack/dispatch/src/validate.ts PRIORITY_NAMES) and #7030's bare
 *  `expedite` / `next-up` band labels. Scale-only values (30, 50, 90) have
 *  no mirror label. Other priority-looking labels in the repo, such as
 *  `priority/medium` or `priority/very-high`, name no profile member, so
 *  they are refused as noncanonical rather than guessed at. */
export const PRIORITY_MIRROR_LABELS: ReadonlyMap<number, string> = new Map([
  [10, "priority/critical"],
  [15, "expedite"],
  [20, "priority/high"],
  [25, "next-up"],
  [40, "priority/normal"],
  [70, "priority/low"],
]);
export function priorityLabelFor(value: number): string | null {
  return PRIORITY_MIRROR_LABELS.get(value) ?? null;
}
const CANONICAL_PRIORITY_LABELS: ReadonlySet<string> = new Set(PRIORITY_MIRROR_LABELS.values());

/** Project #4 Status values a mobile request may name (status-taxonomy.ts, source `project`). */
export const PROJECT_STATUS_VALUES = ["Todo", "Backlog", "In Progress", "In Review", "Done"] as const;
export type ProjectStatus = (typeof PROJECT_STATUS_VALUES)[number];

export type Lifecycle = "open" | "blocked" | "parked" | "completed";

/** What the caller read from the authoritative owners under one work version. */
export interface WorkItemFacts {
  objectRef: string;
  workVersion: string;
  lifecycle: Lifecycle;
  manualOnly: boolean;
  /** Open hard (`blockedBy`) dependencies. Advisory links do not belong here. */
  openHardBlockers: readonly string[];
  /** Current intrinsic urgency in `fleet.queue@1`, or null when untriaged. */
  priority: number | null;
  doNow: boolean;
}

/** The versioned `fleet.owner-priority-policy/v1` view, or null when unreadable. */
export interface OwnerPolicyView {
  policyRef: string;
  ownerPrincipalRef: string;
  protectedMembers: ReadonlySet<string>;
}

/** The server-authenticated principal. Never client-supplied. */
export interface MobilePrincipal {
  principalRef: string;
}

export type MobileVerbRequest =
  | {
      verb: "note.add";
      body: string;
      visibility?: "private" | "public";
      publicMirrorPreviewed?: boolean;
    }
  | { verb: "comment.add"; body: string; replyTo?: string; confirmed?: boolean }
  | {
      verb: "priority.set";
      value: number;
      reason: string;
      ownerPolicyRef?: string;
      /** Priority labels the client shows; must be canonical and agree with `value`. */
      labels?: readonly string[];
      /** Explicit owner action. Only the policy's owner principal may use it. */
      ownerAction?: boolean;
    }
  | { verb: "directive.do_now"; set: boolean; confirmed?: boolean }
  | {
      verb: "status.request";
      target: "ready" | "parked" | "blocked";
      reason?: string;
      checkpoint?: string;
    }
  | { verb: "project.request_status"; status: string };

export type MobileVerb = MobileVerbRequest["verb"];

export const MOBILE_VERBS: readonly MobileVerb[] = [
  "note.add",
  "comment.add",
  "priority.set",
  "directive.do_now",
  "status.request",
  "project.request_status",
];

/** The owners a change may target. Each is an existing owner, not a new store. */
export type ChangeOwner =
  | "fleet.journal"
  | "github.comment"
  | "fleet.priority"
  | "fleet.directive"
  | "fleet.lifecycle"
  | "orchestrator.project_status";

export interface SourceOfTruthChange {
  owner: ChangeOwner;
  op: string;
  objectRef: string;
  fields: Readonly<Record<string, unknown>>;
}

export type DenyCode =
  | "EMPTY_BODY"
  | "PUBLIC_MIRROR_NOT_PREVIEWED"
  | "CONFIRMATION_REQUIRED"
  | "NONCANONICAL_PRIORITY"
  | "CONTRADICTORY_PRIORITY_LABELS"
  | "REASON_REQUIRED"
  | "owner_policy_unavailable"
  | "owner_policy_ref_missing"
  | "OWNER_POLICY_VERSION_CONFLICT"
  | "OWNER_POLICY_PROTECTED"
  | "DO_NOW_HARD_GATE"
  | "LIFECYCLE_TERMINAL"
  | "HARD_BLOCKER_OPEN"
  | "PARK_CHECKPOINT_REQUIRED"
  | "UNSUPPORTED_PROJECT_STATUS"
  | "DONE_REQUIRES_VERIFIED_RECEIPT";

export type MobilePolicyResult =
  | {
      kind: "allow";
      verb: MobileVerb;
      changes: readonly SourceOfTruthChange[];
      /** True for every priority write: the #9200 mapping re-runs after apply. */
      rerunPriorityMapping: boolean;
    }
  | { kind: "deny"; verb: MobileVerb; code: DenyCode; detail: string };

function deny(verb: MobileVerb, code: DenyCode, detail: string): MobilePolicyResult {
  return { kind: "deny", verb, code, detail };
}

function allow(
  verb: MobileVerb,
  changes: readonly SourceOfTruthChange[],
  rerunPriorityMapping = false,
): MobilePolicyResult {
  return { kind: "allow", verb, changes, rerunPriorityMapping };
}

/** Every label that reads as a priority label, canonical or not. */
function priorityish(label: string): boolean {
  const l = label.trim().toLowerCase();
  return (
    l.startsWith("priority") ||
    l.startsWith("prio") ||
    /^p[0-9]$/.test(l) ||
    l === "expedite" ||
    l === "next-up" ||
    l === "urgent"
  );
}

/**
 * The PRI-01 protected-owner gate, in the same order as
 * `check_priority_write`: a readable policy, a cited and current
 * `owner_policy_ref`, then owner-only writes to protected items. Mobile is
 * a routine path, so even the owner's principal must also declare an
 * explicit owner action; a mobile edit without it stays protected.
 */
function checkOwnerPolicy(
  facts: WorkItemFacts,
  request: Extract<MobileVerbRequest, { verb: "priority.set" }>,
  policy: OwnerPolicyView | null,
  principal: MobilePrincipal | null,
): MobilePolicyResult | null {
  const verb = "priority.set";
  if (policy === null) {
    return deny(verb, "owner_policy_unavailable", "owner priority policy is missing or unverifiable; failing closed");
  }
  if (!request.ownerPolicyRef) {
    return deny(verb, "owner_policy_ref_missing", "priority mutation without an owner policy reference");
  }
  if (request.ownerPolicyRef !== policy.policyRef) {
    return deny(
      verb,
      "OWNER_POLICY_VERSION_CONFLICT",
      `request cites ${request.ownerPolicyRef}, current policy is ${policy.policyRef}`,
    );
  }
  if (!policy.protectedMembers.has(facts.objectRef)) return null;
  const isOwner = request.ownerAction === true && principal?.principalRef === policy.ownerPrincipalRef;
  return isOwner
    ? null
    : deny(verb, "OWNER_POLICY_PROTECTED", "protected expedite item: only the owner's explicit action may change it");
}

/**
 * Decide one mobile verb. Returns exactly one policy result: either a
 * denial with a code, or the list of source-of-truth changes the apply
 * would hand to the existing owners.
 */
export function planMobileAction(
  facts: WorkItemFacts,
  request: MobileVerbRequest,
  policy: OwnerPolicyView | null,
  principal: MobilePrincipal | null,
): MobilePolicyResult {
  const ref = facts.objectRef;
  switch (request.verb) {
    case "note.add": {
      if (!request.body.trim()) return deny(request.verb, "EMPTY_BODY", "note body is empty");
      const note: SourceOfTruthChange = {
        owner: "fleet.journal",
        op: "note.append",
        objectRef: ref,
        fields: { visibility: "private", body: request.body },
      };
      if ((request.visibility ?? "private") === "private") return allow(request.verb, [note]);
      if (request.publicMirrorPreviewed !== true) {
        return deny(
          request.verb,
          "PUBLIC_MIRROR_NOT_PREVIEWED",
          "a public mirror must be selected and previewed before it is posted",
        );
      }
      return allow(request.verb, [
        note,
        {
          owner: "github.comment",
          op: "comment.create",
          objectRef: ref,
          fields: { body: request.body, mirrorOf: "note" },
        },
      ]);
    }
    case "comment.add": {
      if (!request.body.trim()) return deny(request.verb, "EMPTY_BODY", "comment body is empty");
      if (request.confirmed !== true) {
        return deny(request.verb, "CONFIRMATION_REQUIRED", "a public comment needs an explicit confirmation");
      }
      return allow(request.verb, [
        {
          owner: "github.comment",
          op: request.replyTo ? "comment.reply" : "comment.create",
          objectRef: ref,
          fields: request.replyTo ? { body: request.body, replyTo: request.replyTo } : { body: request.body },
        },
      ]);
    }
    case "priority.set": {
      if (!QUEUE_V1_GOLDEN.has(request.value)) {
        return deny(
          request.verb,
          "NONCANONICAL_PRIORITY",
          `${request.value} is not a member of ${QUEUE_PROFILE_ID}@${QUEUE_PROFILE_VERSION}; values are never snapped`,
        );
      }
      const shown = [...new Set((request.labels ?? []).map((l) => l.trim()).filter(priorityish))];
      const expected = priorityLabelFor(request.value);
      const noncanonical = shown.filter((l) => !CANONICAL_PRIORITY_LABELS.has(l));
      if (noncanonical.length > 0) {
        return deny(
          request.verb,
          "NONCANONICAL_PRIORITY",
          `noncanonical priority label(s): ${noncanonical.join(", ")}`,
        );
      }
      if (shown.some((l) => l !== expected) || shown.length > 1) {
        return deny(
          request.verb,
          "CONTRADICTORY_PRIORITY_LABELS",
          `labels ${shown.join(", ")} disagree with the requested value ${request.value} (${expected ?? "no mirror label"})`,
        );
      }
      if (!request.reason.trim()) return deny(request.verb, "REASON_REQUIRED", "a reprioritization needs a reason");
      const gate = checkOwnerPolicy(facts, request, policy, principal);
      if (gate) return gate;
      return allow(
        request.verb,
        [
          {
            owner: "fleet.priority",
            op: "priority.write",
            objectRef: ref,
            fields: {
              profile: `${QUEUE_PROFILE_ID}@${QUEUE_PROFILE_VERSION}`,
              before: facts.priority,
              after: request.value,
              reason: request.reason,
              ownerPolicyRef: request.ownerPolicyRef,
            },
          },
        ],
        true,
      );
    }
    case "directive.do_now": {
      if (facts.lifecycle === "completed") {
        return deny(request.verb, "LIFECYCLE_TERMINAL", "completed work takes no directive");
      }
      if (request.set) {
        if (facts.lifecycle === "blocked" || facts.lifecycle === "parked" || facts.manualOnly) {
          return deny(
            request.verb,
            "DO_NOW_HARD_GATE",
            `do-now cannot bypass ${facts.manualOnly ? "manual_only" : facts.lifecycle} state`,
          );
        }
        if (facts.openHardBlockers.length > 0) {
          return deny(request.verb, "DO_NOW_HARD_GATE", `open hard blockers: ${facts.openHardBlockers.join(", ")}`);
        }
        if (request.confirmed !== true) {
          return deny(request.verb, "CONFIRMATION_REQUIRED", "do-now needs an explicit confirmation");
        }
      }
      // Urgency is deliberately absent: a directive never moves the score.
      return allow(request.verb, [
        {
          owner: "fleet.directive",
          op: request.set ? "do_now.set" : "do_now.clear",
          objectRef: ref,
          fields: {},
        },
      ]);
    }
    case "status.request": {
      if (facts.lifecycle === "completed") {
        return deny(request.verb, "LIFECYCLE_TERMINAL", "completed work cannot be moved by a status request");
      }
      if (request.target === "ready") {
        if (facts.openHardBlockers.length > 0) {
          return deny(
            request.verb,
            "HARD_BLOCKER_OPEN",
            `cannot request Ready while hard blockers are open: ${facts.openHardBlockers.join(", ")}`,
          );
        }
        return allow(request.verb, [
          {
            owner: "fleet.lifecycle",
            op: "work.resume",
            objectRef: ref,
            fields: {},
          },
        ]);
      }
      if (!request.reason?.trim()) return deny(request.verb, "REASON_REQUIRED", `${request.target} needs a reason`);
      if (request.target === "parked") {
        if (!request.checkpoint?.trim()) {
          return deny(request.verb, "PARK_CHECKPOINT_REQUIRED", "parking needs a continuation checkpoint");
        }
        return allow(request.verb, [
          {
            owner: "fleet.lifecycle",
            op: "work.park",
            objectRef: ref,
            fields: { reason: request.reason, checkpoint: request.checkpoint },
          },
        ]);
      }
      return allow(request.verb, [
        {
          owner: "fleet.lifecycle",
          op: "work.block",
          objectRef: ref,
          fields: { reason: request.reason },
        },
      ]);
    }
    case "project.request_status": {
      const status = PROJECT_STATUS_VALUES.find((s) => s === request.status);
      if (status === undefined) {
        return deny(
          request.verb,
          "UNSUPPORTED_PROJECT_STATUS",
          `${JSON.stringify(request.status)} is not a Project #4 Status`,
        );
      }
      if (status === "Done") {
        return deny(
          request.verb,
          "DONE_REQUIRES_VERIFIED_RECEIPT",
          "visible Done comes from a verified completion receipt, not a status request",
        );
      }
      return allow(request.verb, [
        {
          owner: "orchestrator.project_status",
          op: "project.request_status",
          objectRef: ref,
          fields: { status },
        },
      ]);
    }
  }
}

/**
 * Turn an allowed plan into the MOB-05 request. The plan travels in the
 * payload, so apply hands the owners exactly what preview showed; the
 * expected work version is the one the facts were read under.
 */
export function toWorkActionRequest(
  facts: WorkItemFacts,
  result: MobilePolicyResult,
  operationKey: string,
): WorkActionRequest {
  if (result.kind !== "allow") {
    throw new Error(`mobile-work-actions: a denied ${result.verb} (${result.code}) never becomes a work action`);
  }
  return {
    operationKey,
    kind: "mobile",
    objectRef: facts.objectRef,
    action: result.verb,
    payload: {
      changes: result.changes,
      rerunPriorityMapping: result.rerunPriorityMapping,
    },
    expectedWorkVersion: facts.workVersion,
  };
}

/**
 * Sync-loop guard. When the projection mirrors an applied priority back
 * as a label, re-reading that label must produce no new write: the label
 * that already matches the stored value is an echo, not a request.
 */
export function inboundPriorityLabelIsEcho(facts: WorkItemFacts, observedLabels: readonly string[]): boolean {
  const shown = [...new Set(observedLabels.map((l) => l.trim()).filter(priorityish))];
  const expected = facts.priority === null ? null : priorityLabelFor(facts.priority);
  if (expected === null) return shown.length === 0;
  return shown.length === 1 && shown[0] === expected;
}
