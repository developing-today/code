import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import {
  evaluateClaimBinding,
  LIFECYCLE_POLICY_VERSION,
  projectLifecycleColumn,
  toClaimInput,
  type BrokerTaskRead,
  type LifecycleColumnResult,
  type LifecycleEvidence,
  type LifecyclePolicyOptions,
  type Read,
  type RuleId,
  type SessionReadback,
} from "../src/work-lifecycle-policy.ts";

// #9469 MOB-02: lifecycle projection, column precedence and truthful claim
// binding. Oracle: the design record
// doc/2026-11-25T19-35-30Z_design_mob02_lifecycle_projection_claim_binding/
// (§4.4 invariants, §5 edge cases, §6 golden fixtures, §7 test list).

const NOW = "2026-11-25T12:00:00Z";
const OBS = "2026-11-25T11:59:00Z";
const STALE_OBS = "2026-11-25T11:50:00Z";
const BOUND = 300_000;
const OPTS: LifecyclePolicyOptions = {
  now: NOW,
  stale_after_ms: { fleet_broker: BOUND, session_adapter: BOUND, github: BOUND, project: BOUND, policy: BOUND },
};
const WORK = { authority_id: "fleet-authority", work_id: "9469" };
const FIXTURES = join(import.meta.dir, "fixtures", "work-lifecycle");

const absent = { state: "absent", reason: "not_sourced" } as const;
const p = <T>(value: T, source: string, observed_at = OBS): Read<T> => ({ state: "present", value, source, observed_at });
const err = (source: string): Read<never> => ({ state: "error", source, observed_at: OBS, error_code: "read_failed" });

const task = (o: Partial<BrokerTaskRead> = {}): BrokerTaskRead => ({
  task_id: "task-9469",
  status: "queued",
  worker_id: null,
  lane_id: null,
  lease_until: null,
  attempts: 0,
  max_attempts: 3,
  manual_only: false,
  park: null,
  block: null,
  binding: { progress_session_id: null, ledger_session_id: "not_read" },
  eligibility: { state: "eligible", reason_codes: [], hard_blocker_refs: [] },
  ...o,
});
const claimed = (o: Partial<BrokerTaskRead> = {}) =>
  task({ status: "claimed", worker_id: "lane-8-claude", lane_id: "8", lease_until: "2026-11-25T12:09:00Z", attempts: 1, ...o });
const bound = (id: string, o: Partial<BrokerTaskRead> = {}) =>
  claimed({ binding: { progress_session_id: id, ledger_session_id: id }, ...o });
const ses = (session_id: string, o: Partial<SessionReadback> = {}): SessionReadback => ({
  session_id,
  provider: "opencode",
  bound_work: WORK,
  state: "active",
  last_activity_at: "2026-11-25T11:58:30Z",
  ...o,
});
const noPolicy = { high_or_above: absent, explicitly_prepared: absent, owner_assigned: absent, triage_required: absent };
const ev = (o: Partial<LifecycleEvidence> = {}): LifecycleEvidence => ({
  work: WORK,
  broker: p<BrokerTaskRead | null>(task(), "fleet_broker"),
  sessions: absent,
  remote: p({ issue_state: "open" as const, prs: [], follow_ups: [] }, "github"),
  project_status: absent,
  completion: p({ verified_terminal_receipt: false, receipt_ref: null }, "completion_policy"),
  policy: noPolicy,
  ...o,
});
const remote = (issue_state: "open" | "closed" | null, prs: { locator: string; state: "open" | "closed" | "merged"; draft: boolean }[] = []) =>
  p({ issue_state, prs, follow_ups: [] as { locator: string; state: "open" | "closed" }[] }, "github");
const receipt = p({ verified_terminal_receipt: true, receipt_ref: "receipt://9469" }, "completion_policy");
const high = { ...noPolicy, high_or_above: p(true, "priority_policy") };

const summary = (r: LifecycleColumnResult) => ({
  column: r.column,
  rule: r.rule,
  reason_codes: r.reason_codes,
  conflicts: r.conflicts.map((c) => c.code),
  stale_facets: r.stale_facets,
});
const run = (e: LifecycleEvidence) => projectLifecycleColumn(e, OPTS);
const clone = <T>(x: T): T => JSON.parse(JSON.stringify(x)) as T;

describe("pm_work_lifecycle_policy/v1", () => {
  test("work_lifecycle_never_infers_done_from_one_facet", () => {
    // Each terminal signal alone. Project "Done" is a conflict-only facet
    // and a status:done label is not an input at all (record §3.2).
    const cases: [string, LifecycleEvidence, boolean][] = [
      ["queue done", ev({ broker: p(task({ status: "done" }), "fleet_broker"), remote: remote(null), completion: absent }), true],
      ["issue closed", ev({ broker: p(null, "fleet_broker"), remote: remote("closed"), completion: absent }), true],
      ["pr merged", ev({ broker: p(null, "fleet_broker"), remote: remote(null, [{ locator: "o/r#1", state: "merged", draft: false }]), completion: absent }), true],
      ["project Done", ev({ broker: p(null, "fleet_broker"), remote: absent, completion: absent, project_status: p("Done", "github_project") }), false],
      ["receipt false", ev({ broker: p(null, "fleet_broker"), remote: absent }), false],
    ];
    for (const [name, e, terminal] of cases) {
      const r = run(e);
      expect(r.column, name).not.toBe("Done");
      if (terminal) {
        expect(r.reason_codes, name).toContain("completion_unverified");
        expect(r.conflicts.map((c) => c.code), name).toContain("completion_unverified");
      }
    }
  });

  test("work_lifecycle_never_infers_in_progress_from_one_facet", () => {
    const cases: [string, LifecycleEvidence][] = [
      ["project In Progress", ev({ project_status: p("In Progress", "github_project") })],
      ["open PR", ev({ remote: remote("open", [{ locator: "o/r#1", state: "open", draft: false }]) })],
      ["draft PR", ev({ remote: remote("open", [{ locator: "o/r#1", state: "open", draft: true }]) })],
      ["fresh claim alone", ev({ broker: p(claimed(), "fleet_broker") })],
      ["broker binding alone", ev({ broker: p(bound("ses_a"), "fleet_broker") })],
      ["adapter session alone", ev({ sessions: p([ses("ses_a")], "session_adapter") })],
      ["binding on queued task", ev({ broker: p(task({ binding: { progress_session_id: "ses_a", ledger_session_id: "ses_a" } }), "fleet_broker"), sessions: p([ses("ses_a")], "session_adapter") })],
      ["no worker on claim", ev({ broker: p(bound("ses_a", { worker_id: null }), "fleet_broker"), sessions: p([ses("ses_a")], "session_adapter") })],
    ];
    for (const [name, e] of cases) expect(run(e).column, name).not.toBe("In Progress");
  });

  test("work_lifecycle_verified_binding_projects_in_progress_and_preserves_facets", () => {
    const e = JSON.parse(readFileSync(join(FIXTURES, "verified-binding-in-progress.evidence.json"), "utf8")) as LifecycleEvidence;
    const before = clone(e);
    const r = run(e);
    expect(e).toEqual(before);
    expect(r.policy_version).toBe(LIFECYCLE_POLICY_VERSION);
    expect(r.column).toBe("In Progress");
    expect(r.claim_binding).toEqual({ claim: "claimed_live", fresh: true, session_binding: "verified", bound_session_id: "ses_a", reason_codes: [] });
    expect(r.conflicts.map((c) => c.code)).toEqual(["project_status_disagrees"]);
    expect(e.project_status).toEqual({ state: "present", value: "Todo", source: "github_project", observed_at: OBS });
    expect(toClaimInput(r.claim_binding, "lane-8-claude", "8")).toEqual({ state: "claimed", fresh: true, session_binding: "verified", worker: "lane-8-claude", lane: "8" });
  });

  test("work_lifecycle_pairwise_conflicts_match_golden_oracle", () => {
    const names = readdirSync(FIXTURES).filter((f) => f.endsWith(".evidence.json")).map((f) => f.replace(".evidence.json", "")).sort();
    expect(names).toEqual([
      "claimed-no-session",
      "hard-blocked-high",
      "parked-owner",
      "pr-merged-issue-open",
      "ready-stale",
      "session-shell-unbound-task",
      "task-done-followup-open",
      "verified-binding-in-progress",
    ]);
    for (const name of names) {
      const e = JSON.parse(readFileSync(join(FIXTURES, `${name}.evidence.json`), "utf8")) as LifecycleEvidence;
      const want = JSON.parse(readFileSync(join(FIXTURES, `${name}.result.json`), "utf8"));
      expect(summary(run(e)), name).toEqual(want);
    }
  });

  test("work_lifecycle_edge_cases_match_golden_oracle", () => {
    const expired = { lease_until: "2026-11-25T11:58:00Z" };
    const rows: [string, LifecycleEvidence, LifecycleColumnResult["column"], RuleId, string[], string[]][] = [
      ["claimed but unbound", ev({ broker: p(claimed(), "fleet_broker") }), "Inbox/Triage", "R6", ["claim_binding_unverified", "no_bound_session"], ["claimed_binding_unknown"]],
      ["broker-bound, no readback", ev({ broker: p(claimed({ binding: { progress_session_id: "ses_a", ledger_session_id: "not_read" } }), "fleet_broker") }), "Inbox/Triage", "R6", ["claim_binding_unverified", "session_readback_absent"], ["claimed_binding_unknown"]],
      ["lease expired, sweep pending", ev({ broker: p(claimed(expired), "fleet_broker") }), "Backlog", "R8", ["claim_lease_expired_awaiting_reclaim"], ["claimed_lease_expired"]],
      ["lease expired at cap", ev({ broker: p(claimed({ ...expired, attempts: 3 }), "fleet_broker") }), "Inbox/Triage", "R6", ["attempt_cap_reached", "claim_lease_expired_awaiting_reclaim"], ["claimed_lease_expired"]],
      ["queued at cap", ev({ broker: p(task({ attempts: 3, eligibility: { state: "ineligible", reason_codes: ["attempt_cap"], hard_blocker_refs: [] } }), "fleet_broker"), policy: high }), "Inbox/Triage", "R6", ["attempt_cap_reached"], []],
      ["failed after cap", ev({ broker: p(task({ status: "failed", attempts: 3 }), "fleet_broker") }), "Inbox/Triage", "R6", ["queue_failed"], []],
      ["person wait (block)", ev({ broker: p(task({ status: "blocked", block: { reason_class: "escalated-to-user", blocked_deps: [], dep_blocked_until: null } }), "fleet_broker") }), "Blocked", "R3", ["queue_blocked", "waiting_on_person"], []],
      ["person wait (park)", ev({ broker: p(task({ status: "parked", park: { park_kind: "relative", wait_kind: "review", reason: "needs answer", checkpoint_ref: null, not_before: null, condition_kind: "person_answer" } }), "fleet_broker") }), "Parked", "R2", ["parked_with_reason", "waiting_on_person"], []],
      ["session asks a question", ev({ broker: p(bound("ses_a"), "fleet_broker"), sessions: p([ses("ses_a", { state: "awaiting_input" })], "session_adapter") }), "In Progress", "R4", ["fresh_authoritative_claim", "session_awaiting_input", "verified_session_binding"], []],
      ["done, PR still open", ev({ broker: p(task({ status: "done" }), "fleet_broker"), remote: remote("closed", [{ locator: "o/r#5", state: "open", draft: false }]), completion: receipt }), "Review", "R5", ["completion_unverified", "pr_awaiting_review"], ["work_terminal_pr_open"]],
      ["done, no receipt, issue open", ev({ broker: p(task({ status: "done" }), "fleet_broker") }), "Review", "R5", ["completion_unverified"], ["completion_unverified", "work_terminal_issue_open"]],
      ["broker bindings disagree", ev({ broker: p(claimed({ binding: { progress_session_id: "ses_a", ledger_session_id: "ses_b" } }), "fleet_broker"), sessions: p([ses("ses_a")], "session_adapter") }), "Inbox/Triage", "R6", ["claim_binding_unverified"], ["binding_session_mismatch", "claimed_binding_unknown"]],
      ["multiple adapter sessions", ev({ broker: p(bound("ses_a"), "fleet_broker"), sessions: p([ses("ses_a"), ses("ses_c")], "session_adapter") }), "In Progress", "R4", ["fresh_authoritative_claim", "verified_session_binding"], ["extra_session_for_work"]],
      ["bound session ended", ev({ broker: p(bound("ses_a"), "fleet_broker"), sessions: p([ses("ses_a", { state: "terminal" })], "session_adapter") }), "Inbox/Triage", "R6", ["claim_binding_unverified"], ["bound_session_terminal", "claimed_binding_unknown"]],
      ["parked, stale active binding", ev({ broker: p(task({ status: "parked", park: { park_kind: "absolute", wait_kind: "timer", reason: "later", checkpoint_ref: null, not_before: null } }), "fleet_broker"), sessions: p([ses("ses_a")], "session_adapter") }), "Parked", "R2", ["parked_with_reason"], ["held_with_active_binding"]],
      ["handed off to the sink", ev({ broker: p(task({ status: "handed_off" }), "fleet_broker") }), "Backlog", "R8", ["handed_off_unbound", "not_ready"], []],
      ["cancelled", ev({ broker: p(task({ status: "cancelled" }), "fleet_broker"), remote: remote("closed") }), "Inbox/Triage", "R6", ["queue_cancelled"], []],
      ["no fleet task", ev({ broker: p(null, "fleet_broker"), project_status: p("Todo", "github_project") }), "Backlog", "R8", ["no_fleet_task"], []],
      ["broker read failed", ev({ broker: err("fleet_broker") }), null, "R0", ["lifecycle_source_error"], []],
      ["unknown stored spelling", ev({ broker: p(task({ status: "in_progress" }), "fleet_broker") }), null, "R0", ["unknown_queue_status"], []],
      ["manual-only, do now", ev({ broker: p(task({ manual_only: true }), "fleet_broker"), policy: high, dispatch_directive: p("do_now", "dispatch") }), "Backlog", "R8", ["manual_only_excluded_from_automatic_claim"], []],
      ["draft PR, no claim", ev({ remote: remote("open", [{ locator: "o/r#7", state: "open", draft: true }]) }), "Backlog", "R8", ["draft_pr_open", "not_ready"], []],
    ];
    for (const [name, e, column, rule, reasons, conflicts] of rows) {
      const r = run(e);
      expect({ column: r.column, rule: r.rule, reason_codes: r.reason_codes, conflicts: r.conflicts.map((c) => c.code) }, name).toEqual({
        column,
        rule,
        reason_codes: [...reasons].sort(),
        conflicts: [...conflicts].sort(),
      });
    }
  });

  test("work_lifecycle_held_precedes_ready_and_backlog", () => {
    const held: [string, BrokerTaskRead, string][] = [
      ["parked", task({ status: "parked", park: { park_kind: "relative", wait_kind: "timer", reason: "later", checkpoint_ref: "c", not_before: null } }), "Parked"],
      ["blocked", task({ status: "blocked", block: { reason_class: "dependency", blocked_deps: ["o/r#3"], dep_blocked_until: null } }), "Blocked"],
      ["hard blocker", task({ eligibility: { state: "ineligible", reason_codes: [], hard_blocker_refs: ["fleet://work/9"] } }), "Blocked"],
    ];
    for (const [name, t, column] of held) {
      for (let bits = 0; bits < 16; bits++) {
        const on = (i: number) => (bits & (1 << i)) !== 0;
        const r = run(ev({
          broker: p(t, "fleet_broker"),
          policy: {
            high_or_above: on(0) ? p(true, "priority_policy") : absent,
            owner_assigned: on(1) ? p(true, "github") : absent,
            explicitly_prepared: on(2) ? p(true, "prep") : absent,
            triage_required: absent,
          },
          dispatch_directive: on(3) ? p("do_now", "dispatch") : absent,
        }));
        expect(r.column, `${name} bits=${bits}`).toBe(column as LifecycleColumnResult["column"]);
        expect(r.reason_codes.includes("held_not_promoted"), `${name} bits=${bits}`).toBe(bits !== 0);
      }
    }
  });

  test("work_lifecycle_stale_evidence_never_promotes", () => {
    const parked = task({ status: "parked", park: { park_kind: "relative", wait_kind: "timer", reason: "later", checkpoint_ref: null, not_before: null } });
    const bases: [string, LifecycleEvidence][] = [
      ["ready", ev({ policy: high })],
      ["in progress", ev({ broker: p(bound("ses_a"), "fleet_broker"), sessions: p([ses("ses_a")], "session_adapter") })],
      ["parked", ev({ broker: p(parked, "fleet_broker"), policy: high })],
      ["backlog", ev()],
      ["done", ev({ broker: p(task({ status: "done" }), "fleet_broker"), remote: remote("closed"), completion: receipt })],
      ["review", ev({ remote: remote("open", [{ locator: "o/r#1", state: "open", draft: false }]) })],
    ];
    const staleKeys = ["broker", "sessions", "remote", "project_status", "completion", "policy.high_or_above", "policy.owner_assigned", "policy.explicitly_prepared"] as const;
    const markStale = (e: LifecycleEvidence, key: (typeof staleKeys)[number]): LifecycleEvidence => {
      const out = clone(e);
      const age = <T>(r: Read<T>): Read<T> => (r.state === "present" ? { ...r, observed_at: STALE_OBS } : r);
      if (key.startsWith("policy.")) {
        const k = key.slice(7) as keyof LifecycleEvidence["policy"];
        out.policy = { ...out.policy, [k]: age(out.policy[k]) };
      } else {
        (out as unknown as Record<string, Read<unknown>>)[key] = age((out as unknown as Record<string, Read<unknown>>)[key] as Read<unknown>);
      }
      return out;
    };
    for (const [name, base] of bases) {
      const was = run(base).column;
      for (const key of staleKeys) {
        const r = run(markStale(base, key));
        const label = `${name} with stale ${key}`;
        if (was !== "In Progress") expect(r.column, label).not.toBe("In Progress");
        if (was !== "Done") expect(r.column, label).not.toBe("Done");
        if (was !== "Ready") expect(r.column, label).not.toBe("Ready");
        if (was === "Parked" || was === "Blocked") expect(r.column, label).toBe(was);
      }
    }
    // Ready and In Progress need fresh evidence (invariant 5).
    expect(run(markStale(bases[0]![1], "broker")).column).toBe("Backlog");
    expect(run(markStale(bases[0]![1], "policy.high_or_above")).column).toBe("Backlog");
    const staleClaim = run(markStale(bases[1]![1], "broker"));
    expect(staleClaim.column).toBe("Inbox/Triage");
    expect(staleClaim.claim_binding.reason_codes).toEqual(["claim_read_stale"]);
    const staleSession = run(markStale(bases[1]![1], "sessions"));
    expect(staleSession.column).toBe("Inbox/Triage");
    expect(staleSession.claim_binding.reason_codes).toEqual(["session_read_stale"]);
    const staleHeld = run(markStale(bases[2]![1], "broker"));
    expect(staleHeld.reason_codes).toContain("held_state_stale");
  });

  test("work_lifecycle_claim_binding_requires_both_sides", () => {
    const good = ev({ broker: p(bound("ses_a"), "fleet_broker"), sessions: p([ses("ses_a")], "session_adapter") });
    expect(evaluateClaimBinding(good, OPTS)).toMatchObject({ claim: "claimed_live", fresh: true, session_binding: "verified" });
    const cases: [string, LifecycleEvidence, Partial<ReturnType<typeof evaluateClaimBinding>>][] = [
      ["1: broker absent", { ...good, broker: absent }, { claim: "unclaimed", fresh: false }],
      ["1: broker stale", { ...good, broker: p(bound("ses_a"), "fleet_broker", STALE_OBS) }, { claim: "claimed_read_stale", fresh: false, reason_codes: ["claim_read_stale"] }],
      ["2: not claimed", { ...good, broker: p(bound("ses_a", { status: "queued" }), "fleet_broker") }, { claim: "unclaimed", fresh: false }],
      ["2: no worker", { ...good, broker: p(bound("ses_a", { worker_id: null }), "fleet_broker") }, { claim: "claimed_lease_expired", fresh: false }],
      ["3: lease lapsed", { ...good, broker: p(bound("ses_a", { lease_until: "2026-11-25T11:58:00Z" }), "fleet_broker") }, { claim: "claimed_lease_expired", fresh: false, reason_codes: ["claim_lease_expired_awaiting_reclaim"] }],
      ["4: no binding", { ...good, broker: p(claimed(), "fleet_broker") }, { fresh: true, session_binding: "unknown", reason_codes: ["no_bound_session"] }],
      ["4: ledger disagrees", { ...good, broker: p(claimed({ binding: { progress_session_id: "ses_a", ledger_session_id: "ses_b" } }), "fleet_broker") }, { fresh: true, session_binding: "unknown" }],
      ["5: sessions absent", { ...good, sessions: absent }, { fresh: true, session_binding: "unknown", reason_codes: ["session_readback_absent"] }],
      ["5: sessions error", { ...good, sessions: err("session_adapter") }, { fresh: true, session_binding: "unknown", reason_codes: ["session_read_error"] }],
      ["5: sessions stale", { ...good, sessions: p([ses("ses_a")], "session_adapter", STALE_OBS) }, { fresh: true, session_binding: "unknown", reason_codes: ["session_read_stale"] }],
      ["6: other work", { ...good, sessions: p([ses("ses_a", { bound_work: { authority_id: "fleet-authority", work_id: "1" } })], "session_adapter") }, { fresh: true, session_binding: "unknown", reason_codes: ["session_readback_absent"] }],
      ["6: terminal", { ...good, sessions: p([ses("ses_a", { state: "terminal" })], "session_adapter") }, { fresh: true, session_binding: "unknown" }],
      ["6: duplicated readback", { ...good, sessions: p([ses("ses_a"), ses("ses_a")], "session_adapter") }, { fresh: true, session_binding: "unknown" }],
    ];
    for (const [name, e, want] of cases) {
      const v = evaluateClaimBinding(e, OPTS);
      expect(v, name).toMatchObject(want);
      expect(v.session_binding === "verified", name).toBe(false);
      expect(run(e).column, name).not.toBe("In Progress");
    }
  });

  test("work_lifecycle_lease_expiry_uses_read_time_not_wall_clock", () => {
    const sessions = p([ses("ses_a")], "session_adapter");
    // Client clock behind the broker cannot revive a lease the read saw lapse.
    const behind = { ...OPTS, now: "2026-11-25T11:58:00Z" };
    const lapsed = ev({ broker: p(bound("ses_a", { lease_until: "2026-11-25T11:58:30Z" }), "fleet_broker"), sessions });
    expect(evaluateClaimBinding(lapsed, behind).claim).toBe("claimed_lease_expired");
    expect(projectLifecycleColumn(lapsed, behind).column).not.toBe("In Progress");
    // Client clock behind the broker with a live lease: still live.
    const live = ev({ broker: p(bound("ses_a"), "fleet_broker"), sessions });
    expect(evaluateClaimBinding(live, behind)).toMatchObject({ claim: "claimed_live", fresh: true, session_binding: "verified" });
    // Client clock ahead of the lease cannot expire a lease the read saw live.
    const ahead = ev({ broker: p(bound("ses_a", { lease_until: "2026-11-25T11:59:30Z" }), "fleet_broker"), sessions });
    expect(evaluateClaimBinding(ahead, OPTS)).toMatchObject({ claim: "claimed_live", fresh: true, session_binding: "verified" });
    expect(run(ahead).column).toBe("In Progress");
  });

  test("work_lifecycle_error_or_unknown_status_yields_null_column", () => {
    const cases: [string, LifecycleEvidence, string][] = [
      ["broker error", ev({ broker: err("fleet_broker") }), "lifecycle_source_error"],
      ["unknown spelling", ev({ broker: p(task({ status: "running" }), "fleet_broker") }), "unknown_queue_status"],
      ["nothing sourced", ev({ broker: absent, remote: absent, completion: absent }), "nothing_sourced"],
    ];
    for (const [name, e, reason] of cases) {
      const r = run(e);
      expect(r.column, name).toBeNull();
      expect(r.rule, name).toBe("R0");
      expect(r.reason_codes, name).toEqual([reason]);
    }
  });

  test("work_lifecycle_absent_fleet_task_is_not_ready", () => {
    const noTask: Read<BrokerTaskRead | null>[] = [p(null, "fleet_broker"), absent];
    for (const broker of noTask) {
      for (const policy of [noPolicy, high, { ...noPolicy, owner_assigned: p(true, "github"), explicitly_prepared: p(true, "prep") }]) {
        const r = run(ev({ broker, policy, sessions: p([ses("ses_a")], "session_adapter") }));
        expect(r.column).not.toBe("Ready");
        expect(r.column).not.toBe("In Progress");
        expect(r.reason_codes).toContain("no_fleet_task");
      }
    }
  });

  test("work_lifecycle_preserves_source_spellings", () => {
    for (const directive of ["expedite=15", "next-up=25", "expedite", "Do Now", "DO_NOW"]) {
      const e = ev({ broker: p(task({ status: "parked", park: { park_kind: "relative", wait_kind: "timer", reason: "x", checkpoint_ref: null, not_before: null } }), "fleet_broker"), dispatch_directive: p(directive, "dispatch") });
      expect(run(e).reason_codes, directive).not.toContain("held_not_promoted");
      expect(run(ev({ dispatch_directive: p(directive, "dispatch") })).column, directive).toBe("Backlog");
    }
    const odd = ev({ broker: p(task({ status: "parked", park: { park_kind: "lunar", wait_kind: "moonrise", reason: "x", checkpoint_ref: null, not_before: null, condition_kind: "eclipse" } }), "fleet_broker") });
    const oddBefore = clone(odd);
    expect(summary(run(odd))).toMatchObject({ column: "Parked", reason_codes: ["parked_with_reason"] });
    expect(odd).toEqual(oddBefore);
    const blocked = ev({ broker: p(task({ status: "blocked", block: { reason_class: "mystery-class", blocked_deps: [], dep_blocked_until: null } }), "fleet_broker") });
    expect(run(blocked).reason_codes).toEqual(["queue_blocked"]);
    const unknown = ev({ broker: p(task({ status: "in_progress" }), "fleet_broker") });
    run(unknown);
    expect(unknown.broker.state === "present" && unknown.broker.value?.status).toBe("in_progress");
  });

  test("work_lifecycle_matrix_invariants", () => {
    const statuses: (string | "absent" | "error")[] = ["absent", "error", "queued", "claimed", "parked", "blocked", "handed_off", "done", "cancelled", "failed", "in_progress"];
    const leases = ["none", "live", "expired"] as const;
    const bindings = ["none", "broker-only", "verified", "mismatch"] as const;
    const adapters = ["absent", "error", "stale", "matching", "other", "terminal"] as const;
    const remotes = ["none", "issue-open", "issue-closed", "pr-open", "pr-draft", "pr-merged"] as const;
    const receipts = [false, true];
    const policies = ["none", "high", "owner", "prepared", "triage"] as const;
    const freshness = ["live", "stale"] as const;
    const terminalSignals = (e: LifecycleEvidence) => {
      const t = e.broker.state === "present" ? e.broker.value : null;
      const r = e.remote.state === "present" ? e.remote.value : null;
      const rec = e.completion.state === "present" && e.completion.value.verified_terminal_receipt;
      return rec || t?.status === "done" || r?.issue_state === "closed" || (r?.prs.some((x) => x.state === "merged") ?? false);
    };
    let count = 0;
    for (const status of statuses)
      for (const lease of leases)
        for (const binding of bindings)
          for (const adapter of adapters)
            for (const rem of remotes)
              for (const rec of receipts)
                for (const pol of policies)
                  for (const fresh of freshness) {
                    const obs = fresh === "live" ? OBS : STALE_OBS;
                    const sid = binding === "none" ? null : "ses_a";
                    const t = task({
                      status: status === "absent" || status === "error" ? "queued" : status,
                      worker_id: lease === "none" ? null : "lane-8-claude",
                      lease_until: lease === "none" ? null : lease === "live" ? "2026-11-25T12:09:00Z" : "2026-11-25T11:00:00Z",
                      binding: { progress_session_id: sid, ledger_session_id: binding === "mismatch" ? "ses_b" : binding === "verified" ? "ses_a" : "not_read" },
                      park: status === "parked" ? { park_kind: "relative", wait_kind: "timer", reason: "r", checkpoint_ref: null, not_before: null } : null,
                    });
                    const sessions: Read<SessionReadback[]> =
                      adapter === "absent" ? absent
                      : adapter === "error" ? err("session_adapter")
                      : p(
                          adapter === "other" ? [ses("ses_z")] : [ses("ses_a", { state: adapter === "terminal" ? "terminal" : "active" })],
                          "session_adapter",
                          adapter === "stale" ? STALE_OBS : OBS,
                        );
                    const e = ev({
                      broker: status === "absent" ? absent : status === "error" ? err("fleet_broker") : p(t, "fleet_broker", obs),
                      sessions,
                      remote:
                        rem === "none" ? remote(null)
                        : rem === "issue-open" ? remote("open")
                        : rem === "issue-closed" ? remote("closed")
                        : remote(null, [{ locator: "o/r#9", state: rem === "pr-merged" ? "merged" : "open", draft: rem === "pr-draft" }]),
                      completion: rec ? receipt : absent,
                      policy: {
                        high_or_above: pol === "high" ? p(true, "priority_policy", obs) : absent,
                        owner_assigned: pol === "owner" ? p(true, "github", obs) : absent,
                        explicitly_prepared: pol === "prepared" ? p(true, "prep", obs) : absent,
                        triage_required: pol === "triage" ? p(true, "triage", obs) : absent,
                      },
                    });
                    const r = run(e);
                    count++;
                    const tk = e.broker.state === "present" ? e.broker.value : null;
                    const label = JSON.stringify({ status, lease, binding, adapter, rem, rec, pol, fresh });
                    if (r.column === "In Progress") {
                      if (!r.claim_binding.fresh || r.claim_binding.session_binding !== "verified" || fresh === "stale" || adapter !== "matching") throw new Error(`invariant 1/5: ${label}`);
                    }
                    if (r.column === "Done") {
                      const rm = e.remote.state === "present" ? e.remote.value : null;
                      if (!rec || rm?.issue_state === "open" || rm?.prs.some((x) => x.state === "open") || ["queued", "claimed", "parked", "blocked", "handed_off"].includes(tk?.status ?? "")) {
                        throw new Error(`invariant 2: ${label}`);
                      }
                    }
                    if (r.column !== null && ["Ready", "Backlog", "Inbox/Triage", "Review", "In Progress"].includes(r.column)) {
                      if (tk && (tk.status === "parked" || tk.status === "blocked" || tk.eligibility.hard_blocker_refs.length > 0)) throw new Error(`invariant 3: ${label}`);
                    }
                    if (r.column === "Ready" && (fresh === "stale" || !tk || tk.manual_only || tk.eligibility.state !== "eligible")) throw new Error(`invariant 4/5: ${label}`);
                    if (r.column === "Done" && !terminalSignals(e)) throw new Error(`invariant 7: ${label}`);
                    if ((status === "error" || status === "in_progress") !== (r.column === null)) throw new Error(`null column: ${label}`);
                    if (r.reason_codes.join() !== [...r.reason_codes].sort().join()) throw new Error(`unsorted reasons: ${label}`);
                  }
    expect(count).toBe(11 * 3 * 4 * 6 * 6 * 2 * 5 * 2);
  });

  test("work_lifecycle_is_deterministic_and_pure", () => {
    const e = ev({
      broker: p(bound("ses_a", { eligibility: { state: "eligible", reason_codes: ["b", "a"], hard_blocker_refs: [] } }), "fleet_broker"),
      sessions: p([ses("ses_a"), ses("ses_c"), ses("ses_b")], "session_adapter"),
      remote: p({
        issue_state: "open" as const,
        prs: [{ locator: "o/r#3", state: "open" as const, draft: false }, { locator: "o/r#1", state: "open" as const, draft: true }],
        follow_ups: [{ locator: "o/r#9", state: "open" as const }, { locator: "o/r#8", state: "closed" as const }],
      }, "github"),
      project_status: p("Done", "github_project"),
      completion: receipt,
    });
    const reverseKeys = (x: unknown): unknown =>
      Array.isArray(x) ? x.map(reverseKeys)
      : x !== null && typeof x === "object" ? Object.fromEntries(Object.entries(x as Record<string, unknown>).reverse().map(([k, v]) => [k, reverseKeys(v)]))
      : x;
    const shuffled = reverseKeys(clone(e)) as LifecycleEvidence;
    if (shuffled.sessions.state === "present") shuffled.sessions.value.reverse();
    if (shuffled.remote.state === "present") {
      shuffled.remote.value.prs.reverse();
      shuffled.remote.value.follow_ups.reverse();
    }
    const before = JSON.stringify(e);
    const a = JSON.stringify(run(e));
    expect(JSON.stringify(run(e))).toBe(a);
    expect(JSON.stringify(run(shuffled))).toBe(a);
    expect(JSON.stringify(e)).toBe(before);
    const source = readFileSync(join(import.meta.dir, "..", "src", "work-lifecycle-policy.ts"), "utf8");
    expect(source.includes("Date.now(")).toBe(false);
    expect(/new Date\(\s*\)/.test(source)).toBe(false);
  });
});
