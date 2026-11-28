// Lifecycle projection, column precedence and truthful claim binding
// (#9469, MOB-02 of the #9201 PM projection design). The design record is
// doc/2026-11-25T19-35-30Z_design_mob02_lifecycle_projection_claim_binding/.
//
// Two pure functions decide one board column per work item from separate
// lifecycle facets under one written precedence rule (R0..R8, record D3),
// and decide whether a claim may be shown as active work (record §3).
// They fetch nothing, read no clock and hold no state: `opts.now` and the
// staleness bounds are injected. Same input, byte-identical output.
//
// The rules that matter most:
// - In Progress needs BOTH a fresh live broker claim AND an adapter
//   readback confirming the same session is bound to this work. A label,
//   Project status, open PR, claim alone or session alone never counts.
// - Done needs the completion policy's verified terminal receipt and no
//   present facet contradicting it. One terminal signal alone is Review
//   with `completion_unverified`, never Done and never Backlog.
// - Parked and Blocked win over every promoting input (urgency, owner,
//   preparation, do_now). Stale evidence may keep work held but never
//   promotes it (record D4).
// - A failed broker read is a null column, not an empty queue (D5, D6).

import type { BoardColumn, ClaimInput } from "./work-projection.ts";

export const LIFECYCLE_POLICY_VERSION = "pm_work_lifecycle_policy/v1";

export interface LifecyclePolicyOptions {
  /** ISO-8601, injected; the module never reads the wall clock. */
  now: string;
  stale_after_ms: {
    fleet_broker: number;
    session_adapter: number;
    github: number;
    project: number;
    policy: number;
  };
}

export type Read<T> =
  | { state: "present"; value: T; source: string; observed_at: string; freshness_hint?: "live" | "stale" | "fixtures" }
  | { state: "absent"; reason: "not_sourced" | "source_has_no_value" }
  | { state: "error"; source: string; observed_at: string | null; error_code: string };

export interface BrokerTaskRead {
  task_id: string;
  /** Stored spelling, verbatim; parsed against the eight known values. */
  status: string;
  worker_id: string | null;
  lane_id: string | null;
  lease_until: string | null;
  attempts: number;
  max_attempts: number;
  manual_only: boolean;
  park: {
    park_kind: string | null;
    wait_kind: string | null;
    reason: string | null;
    checkpoint_ref: string | null;
    not_before: string | null;
    /** The park's ready-condition kind (`person_answer`, `time`, ...),
     *  carried so a person wait is visible; record §5 "Waiting on a
     *  person (park)". Optional: older reads do not carry it. */
    condition_kind?: string | null;
  } | null;
  block: { reason_class: string | null; blocked_deps: string[]; dep_blocked_until: string | null } | null;
  binding: { progress_session_id: string | null; ledger_session_id: string | null | "not_read" };
  eligibility: { state: "eligible" | "ineligible" | "manual_only"; reason_codes: string[]; hard_blocker_refs: string[] };
}

export interface SessionReadback {
  session_id: string;
  provider: string;
  bound_work: { authority_id: string; work_id: string } | null;
  state: "active" | "idle" | "awaiting_input" | "terminal" | "unknown";
  last_activity_at: string | null;
}

export interface RemoteRead {
  /** null: the work has no own issue. */
  issue_state: "open" | "closed" | null;
  prs: { locator: string; state: "open" | "closed" | "merged"; draft: boolean }[];
  /** #9070 spawned-by children. */
  follow_ups: { locator: string; state: "open" | "closed" }[];
}

export interface LifecycleEvidence {
  work: { authority_id: string; work_id: string };
  /** present with a null value: the broker answered and has no task. */
  broker: Read<BrokerTaskRead | null>;
  sessions: Read<SessionReadback[]>;
  remote: Read<RemoteRead>;
  /** Conflict reporting only; never decides the column (record D2). */
  project_status: Read<string>;
  completion: Read<{ verified_terminal_receipt: boolean; receipt_ref: string | null }>;
  policy: {
    /** #9200-owned predicate; absent until #9200 defines it. */
    high_or_above: Read<boolean>;
    explicitly_prepared: Read<boolean>;
    owner_assigned: Read<boolean>;
    triage_required: Read<boolean>;
  };
  /** Dispatch directive spelling, verbatim. Only `do_now` is a promoting
   *  input, and promoting inputs never move held work (invariant 6);
   *  `expedite` is a priority, not `do_now` (record D8). */
  dispatch_directive?: Read<string>;
}

export interface ClaimBindingVerdict {
  claim: "unclaimed" | "claimed_live" | "claimed_lease_expired" | "claimed_read_stale" | "unknown";
  /** Record §3.1 steps 1-3. */
  fresh: boolean;
  /** Record §3.1 steps 4-6. */
  session_binding: "verified" | "unknown";
  bound_session_id: string | null;
  reason_codes: string[];
}

export type RuleId = "R0" | "R1" | "R2" | "R3" | "R4" | "R5" | "R6" | "R7" | "R8";

export interface LifecycleConflict {
  code: string;
  detail: string;
}

export interface LifecycleColumnResult {
  policy_version: typeof LIFECYCLE_POLICY_VERSION;
  column: BoardColumn | null;
  rule: RuleId;
  reason_codes: string[];
  conflicts: LifecycleConflict[];
  stale_facets: string[];
  claim_binding: ClaimBindingVerdict;
}

const QUEUE_STATUSES = new Set(["queued", "claimed", "parked", "blocked", "handed_off", "done", "cancelled", "failed"]);
const OPEN_QUEUE_STATUSES = new Set(["queued", "claimed", "parked", "blocked", "handed_off"]);
const HELD_STATUSES = new Set(["parked", "blocked"]);
// fleet-queue person_wait.rs: PERSON_WAIT_CLASSES / PERSON_WAIT_CONDITION_KINDS.
const PERSON_WAIT_CLASSES = new Set(["escalated-to-user", "drew_decision", "person-answer"]);
const PERSON_WAIT_CONDITION_KINDS = new Set(["person_answer"]);

// Project #4 Status names that agree with a column. "Todo" covers both
// Backlog and Ready; anything unlisted disagrees.
const PROJECT_STATUS_COLUMNS: Record<string, BoardColumn[]> = {
  "todo": ["Backlog", "Ready"],
  "backlog": ["Backlog"],
  "ready": ["Ready"],
  "in progress": ["In Progress"],
  "review": ["Review"],
  "in review": ["Review"],
  "blocked": ["Blocked"],
  "parked": ["Parked"],
  "done": ["Done"],
  "triage": ["Inbox/Triage"],
  "inbox": ["Inbox/Triage"],
  "inbox/triage": ["Inbox/Triage"],
};

type Present<T> = Extract<Read<T>, { state: "present" }>;

const present = <T>(r: Read<T> | undefined): r is Present<T> => r !== undefined && r.state === "present";

function isStale<T>(r: Read<T> | undefined, boundMs: number, nowMs: number): boolean {
  if (!present(r)) return false;
  if (r.freshness_hint === "stale") return true;
  const observed = Date.parse(r.observed_at);
  return !Number.isFinite(observed) || observed < nowMs - boundMs;
}

const sameWork = (a: { authority_id: string; work_id: string } | null, b: { authority_id: string; work_id: string }) =>
  a !== null && a.authority_id === b.authority_id && a.work_id === b.work_id;

const sorted = (codes: Iterable<string>) => [...new Set(codes)].sort();

interface ClaimAnalysis {
  verdict: ClaimBindingVerdict;
  conflicts: LifecycleConflict[];
}

function analyzeClaim(e: LifecycleEvidence, o: LifecyclePolicyOptions): ClaimAnalysis {
  const nowMs = Date.parse(o.now);
  const conflicts: LifecycleConflict[] = [];
  const reasons: string[] = [];
  const verdict = (claim: ClaimBindingVerdict["claim"], fresh = false, binding: "verified" | "unknown" = "unknown", bound: string | null = null): ClaimAnalysis => ({
    verdict: { claim, fresh, session_binding: binding, bound_session_id: bound, reason_codes: sorted(reasons) },
    conflicts,
  });

  const broker = e.broker;
  if (broker.state === "error") return verdict("unknown");
  if (broker.state === "absent" || broker.value === null) return verdict("unclaimed");
  const task = broker.value;
  if (!QUEUE_STATUSES.has(task.status)) return verdict("unknown");
  if (task.status !== "claimed") return verdict("unclaimed");

  // Step 3: liveness is judged at the read's own observation time, never
  // the client clock (FDR-PMO-003), mirroring verb_bind's own test.
  const observedMs = Date.parse(broker.observed_at);
  const leaseMs = task.lease_until === null ? Number.NaN : Date.parse(task.lease_until);
  const leaseLive = task.worker_id !== null && Number.isFinite(leaseMs) && leaseMs > observedMs;
  if (!leaseLive) {
    if (task.lease_until !== null) {
      conflicts.push({ code: "claimed_lease_expired", detail: `claimed task ${task.task_id} has lease_until at or before the broker read` });
    }
    reasons.push("claim_lease_expired_awaiting_reclaim");
    return verdict("claimed_lease_expired");
  }
  if (isStale(broker, o.stale_after_ms.fleet_broker, nowMs)) {
    reasons.push("claim_read_stale");
    return verdict("claimed_read_stale");
  }

  // Steps 4-6: the broker names exactly one bound session and a fresh
  // adapter readback confirms it is bound to this work and not ended.
  const bound = task.binding.progress_session_id;
  if (bound === null) {
    reasons.push("no_bound_session");
    return verdict("claimed_live", true);
  }
  const ledger = task.binding.ledger_session_id;
  if (ledger !== "not_read" && ledger !== bound) {
    if (ledger !== null) {
      conflicts.push({ code: "binding_session_mismatch", detail: `broker binding names ${bound} in progress and ${ledger} in the lane ledger` });
    }
    return verdict("claimed_live", true);
  }
  const sessions = e.sessions;
  if (sessions.state === "error") {
    reasons.push("session_read_error");
    return verdict("claimed_live", true);
  }
  if (sessions.state === "absent") {
    reasons.push("session_readback_absent");
    return verdict("claimed_live", true);
  }
  if (isStale(sessions, o.stale_after_ms.session_adapter, nowMs)) {
    reasons.push("session_read_stale");
    return verdict("claimed_live", true);
  }
  const forBound = sessions.value.filter((s) => s.session_id === bound);
  const confirming = forBound.filter((s) => sameWork(s.bound_work, e.work) && s.state !== "terminal");
  if (confirming.length === 1) return verdict("claimed_live", true, "verified", bound);
  if (forBound.some((s) => s.state === "terminal")) {
    conflicts.push({ code: "bound_session_terminal", detail: `the bound session ${bound} is reported terminal by its adapter` });
    return verdict("claimed_live", true);
  }
  const others = sessions.value.filter((s) => s.session_id !== bound && sameWork(s.bound_work, e.work));
  if (others.length > 0) {
    conflicts.push({ code: "binding_session_mismatch", detail: `broker binds ${bound}; the adapter binds ${others.map((s) => s.session_id).sort().join(", ")}` });
    return verdict("claimed_live", true);
  }
  reasons.push("session_readback_absent");
  return verdict("claimed_live", true);
}

export function evaluateClaimBinding(e: LifecycleEvidence, o: LifecyclePolicyOptions): ClaimBindingVerdict {
  return analyzeClaim(e, o).verdict;
}

export function toClaimInput(v: ClaimBindingVerdict, worker?: string, lane?: string): ClaimInput {
  const out: ClaimInput = { state: v.claim === "unclaimed" ? "unclaimed" : "claimed", fresh: v.fresh, session_binding: v.session_binding };
  if (worker !== undefined) out.worker = worker;
  if (lane !== undefined) out.lane = lane;
  return out;
}

const truthy = (r: Read<boolean> | undefined) => present(r) && r.value === true;

export function projectLifecycleColumn(e: LifecycleEvidence, o: LifecyclePolicyOptions): LifecycleColumnResult {
  const nowMs = Date.parse(o.now);
  const { verdict, conflicts: claimConflicts } = analyzeClaim(e, o);
  const reasons = new Set<string>(verdict.reason_codes);
  const conflicts = new Map<string, LifecycleConflict>();
  const conflict = (code: string, detail: string) => {
    if (!conflicts.has(code)) conflicts.set(code, { code, detail });
  };
  for (const c of claimConflicts) conflict(c.code, c.detail);

  // Staleness per facet (record D4); error facets contribute nothing.
  const bounds = o.stale_after_ms;
  const staleFacets: string[] = [];
  const brokerStale = isStale(e.broker, bounds.fleet_broker, nowMs);
  if (brokerStale) staleFacets.push("broker");
  if (isStale(e.sessions, bounds.session_adapter, nowMs)) staleFacets.push("sessions");
  if (isStale(e.remote, bounds.github, nowMs)) staleFacets.push("remote");
  if (isStale(e.project_status, bounds.project, nowMs)) staleFacets.push("project_status");
  if (isStale(e.completion, bounds.fleet_broker, nowMs)) staleFacets.push("completion");
  const policyKeys = ["high_or_above", "explicitly_prepared", "owner_assigned", "triage_required"] as const;
  for (const key of policyKeys) {
    if (isStale(e.policy[key], bounds.policy, nowMs)) staleFacets.push(`policy.${key}`);
  }
  if (isStale(e.dispatch_directive, bounds.policy, nowMs)) staleFacets.push("dispatch_directive");

  const finish = (column: BoardColumn | null, rule: RuleId): LifecycleColumnResult => {
    if (column !== null && present(e.project_status)) {
      const agrees = PROJECT_STATUS_COLUMNS[e.project_status.value.trim().toLowerCase()] ?? [];
      if (!agrees.includes(column)) {
        conflict("project_status_disagrees", `Project status disagrees with the projected ${column} column`);
      }
    }
    return {
      policy_version: LIFECYCLE_POLICY_VERSION,
      column,
      rule,
      reason_codes: sorted(reasons),
      conflicts: [...conflicts.values()].sort((a, b) => (a.code < b.code ? -1 : a.code > b.code ? 1 : 0)),
      stale_facets: staleFacets.sort(),
      claim_binding: verdict,
    };
  };

  // R0: a failed or unreadable broker read, or nothing sourced at all.
  if (e.broker.state === "error") {
    reasons.add("lifecycle_source_error");
    return finish(null, "R0");
  }
  const task = present(e.broker) ? e.broker.value : null;
  if (task !== null && !QUEUE_STATUSES.has(task.status)) {
    reasons.add("unknown_queue_status");
    return finish(null, "R0");
  }
  const facets: (Read<unknown> | undefined)[] = [e.broker, e.sessions, e.remote, e.project_status, e.completion, ...policyKeys.map((k) => e.policy[k]), e.dispatch_directive];
  if (facets.every((f) => f === undefined || f.state === "absent")) {
    reasons.add("nothing_sourced");
    return finish(null, "R0");
  }

  const status = task?.status ?? null;
  const remote = present(e.remote) ? e.remote.value : null;
  const sessions = present(e.sessions) ? e.sessions.value : [];
  const receipt = present(e.completion) && e.completion.value.verified_terminal_receipt === true;
  const prOpen = remote?.prs.filter((p) => p.state === "open") ?? [];
  const prMerged = remote?.prs.some((p) => p.state === "merged") ?? false;
  const issueOpen = remote?.issue_state === "open";
  const issueClosed = remote?.issue_state === "closed";
  const queueDone = status === "done";
  // A closed issue is a terminal signal unless the queue itself says the
  // work was cancelled or failed (record §5 "Cancelled").
  const terminalSignal = queueDone || prMerged || (issueClosed && status !== "cancelled" && status !== "failed");
  const hardBlockers = task?.eligibility.hard_blocker_refs ?? [];
  const boundWorkSessions = sessions.filter((s) => sameWork(s.bound_work, e.work) && s.state !== "terminal");
  const brokerBinding = task?.binding.progress_session_id ?? null;

  // Facet conflicts (record D7, MOB-01 D6). Details are fixed text plus refs.
  if (issueOpen && prMerged) conflict("issue_open_pr_merged", "the own issue is open while a linked PR is merged");
  if ((queueDone || receipt) && issueOpen) conflict("work_terminal_issue_open", "the work reads terminal while its own issue is open");
  if ((queueDone || receipt) && prOpen.length > 0) {
    conflict("work_terminal_pr_open", `the work reads terminal while ${prOpen.map((p) => p.locator).sort().join(", ")} is open`);
  }
  if (issueClosed && status !== null && OPEN_QUEUE_STATUSES.has(status)) {
    conflict("issue_closed_work_open", `the own issue is closed while the queue task is ${status}`);
  }
  if (terminalSignal && !receipt) conflict("completion_unverified", "a terminal signal is present without a verified terminal receipt");
  if (brokerBinding !== null && status !== null && OPEN_QUEUE_STATUSES.has(status) && status !== "claimed") {
    conflict("binding_without_claim", `the broker binds ${brokerBinding} to a task that is ${status}, not claimed`);
  }
  const held = status !== null && HELD_STATUSES.has(status);
  if (held && boundWorkSessions.length > 0) {
    conflict("held_with_active_binding", `${boundWorkSessions.map((s) => s.session_id).sort().join(", ")} is bound to ${status} work`);
  }
  if (!held && brokerBinding === null && boundWorkSessions.length > 0) {
    conflict("session_shell_unbound", `${boundWorkSessions.map((s) => s.session_id).sort().join(", ")} names this work but the broker has no binding`);
  }
  if (boundWorkSessions.length > 1) {
    conflict("extra_session_for_work", `${boundWorkSessions.length} adapter sessions name this work`);
  }

  // Informational reasons, added wherever their condition holds.
  const waitingOnPerson =
    (status === "blocked" && task?.block?.reason_class != null && PERSON_WAIT_CLASSES.has(task.block.reason_class)) ||
    (status === "parked" && task?.park?.condition_kind != null && PERSON_WAIT_CONDITION_KINDS.has(task.park.condition_kind));
  if (waitingOnPerson) reasons.add("waiting_on_person");
  if (remote?.prs.some((p) => p.state === "open" && p.draft)) reasons.add("draft_pr_open");
  if (status === "handed_off" && task?.worker_id === null && brokerBinding === null) reasons.add("handed_off_unbound");
  if (remote?.follow_ups.some((f) => f.state === "open")) reasons.add("follow_up_open_elsewhere");

  const promoting = truthy(e.policy.high_or_above) || truthy(e.policy.owner_assigned) || truthy(e.policy.explicitly_prepared) ||
    (present(e.dispatch_directive) && e.dispatch_directive.value === "do_now");

  // R1: verified receipt and no present facet contradicting it.
  const contradicted = issueOpen || prOpen.length > 0 || (status !== null && OPEN_QUEUE_STATUSES.has(status));
  if (receipt && !contradicted) {
    reasons.add("verified_terminal_receipt");
    return finish("Done", "R1");
  }

  // R2, R3: held work keeps its state; stale evidence may keep it held.
  if (status === "parked" || status === "blocked" || hardBlockers.length > 0) {
    if (promoting) reasons.add("held_not_promoted");
    if (brokerStale) reasons.add("held_state_stale");
    if (status === "parked") {
      reasons.add("parked_with_reason");
      return finish("Parked", "R2");
    }
    if (status === "blocked") reasons.add("queue_blocked");
    if (hardBlockers.length > 0) reasons.add("hard_blockers_present");
    return finish("Blocked", "R3");
  }

  // R4: fresh live claim and a verified session binding, both required.
  if (verdict.fresh && verdict.session_binding === "verified") {
    reasons.add("fresh_authoritative_claim");
    reasons.add("verified_session_binding");
    const boundSession = sessions.find((s) => s.session_id === verdict.bound_session_id);
    if (boundSession?.state === "awaiting_input") reasons.add("session_awaiting_input");
    return finish("In Progress", "R4");
  }

  // R6 conditions are computed before R5 so that, when R5 wins, the R6
  // reasons (and the unverified-claim conflict) are still listed.
  const r6: string[] = [];
  if (truthy(e.policy.triage_required)) r6.push("triage_required");
  if (status === "failed") r6.push("queue_failed");
  if (status === "cancelled") r6.push("queue_cancelled");
  if (task !== null && task.attempts >= task.max_attempts && (status === "queued" || verdict.claim === "claimed_lease_expired")) {
    r6.push("attempt_cap_reached");
  }
  if ((verdict.claim === "claimed_live" || verdict.claim === "claimed_read_stale") && verdict.session_binding === "unknown") {
    r6.push("claim_binding_unverified");
    conflict("claimed_binding_unknown", `task ${task?.task_id ?? ""} is claimed but its session binding is not verified`);
  }

  // R5: an open, non-draft PR, or a terminal signal without R1.
  const reviewPr = prOpen.some((p) => !p.draft);
  if (reviewPr || terminalSignal) {
    if (reviewPr) reasons.add("pr_awaiting_review");
    if (terminalSignal) reasons.add("completion_unverified");
    for (const r of r6) reasons.add(r);
    return finish("Review", "R5");
  }

  if (r6.length > 0) {
    for (const r of r6) reasons.add(r);
    return finish("Inbox/Triage", "R6");
  }

  // R7: a fresh eligible broker read, not manual-only, plus at least one
  // fresh promoting policy fact. A stale read never fires R7.
  const manualOnly = task !== null && (task.manual_only || task.eligibility.state === "manual_only");
  const eligible = task !== null && task.eligibility.state === "eligible";
  const freshPolicy = (key: "high_or_above" | "explicitly_prepared" | "owner_assigned") =>
    truthy(e.policy[key]) && !isStale(e.policy[key], bounds.policy, nowMs);
  const readyReasons = [
    freshPolicy("high_or_above") ? "eligible_high_or_above" : null,
    freshPolicy("explicitly_prepared") ? "explicitly_prepared" : null,
    freshPolicy("owner_assigned") ? "owner_assigned" : null,
  ].filter((r): r is string => r !== null);
  if (eligible && !manualOnly && !brokerStale && readyReasons.length > 0) {
    for (const r of readyReasons) reasons.add(r);
    return finish("Ready", "R7");
  }
  if (eligible && brokerStale) reasons.add("eligibility_stale");

  // R8: everything else.
  if (task === null) reasons.add("no_fleet_task");
  else if (manualOnly) reasons.add("manual_only_excluded_from_automatic_claim");
  else if (verdict.claim === "claimed_lease_expired") reasons.add("claim_lease_expired_awaiting_reclaim");
  else reasons.add("not_ready");
  return finish("Backlog", "R8");
}
