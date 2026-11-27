import { describe, expect, test } from "bun:test";
import { projectWorkView, type WorkProjectionInput } from "../src/work-projection.ts";

// #9468 MOB-01 — versioned canonical work read model and provenance.
// Oracle: the #9201 design §3.1 rules, exercised as a deterministic
// matrix over independent GitHub/Project/queue/session states.
// Golden shape: `pm_work_view/v1`; each fact carries source/revision/
// freshness or an explicit absent/stale/error state; unknown remains
// absent; stale/errors are never zero.

const GEN = "2026-11-25T12:00:00Z";
const base: WorkProjectionInput = {
  work: { authority_id: "fleet-authority", work_id: "9468" },
  generated_at: GEN,
};

describe("pm_work_view/v1 golden shape and provenance", () => {
  test("work_view_v1_shape_and_per_fact_provenance", () => {
    const view = projectWorkView({
      ...base,
      revisions: { fleet_work_version: "7", github_revision: "sha-1" },
      intrinsic_urgency: { value: "450", source: "fleet-policy", revision: "r1", observed_at: GEN },
      workflow_state: { value: "open", source: "fleet-lifecycle" },
      project_status: { value: "Todo", source: "github_project_4" },
    });
    expect(view.schema_version).toBe("pm_work_view/v1");
    expect(view.work).toEqual({ authority_id: "fleet-authority", work_id: "9468" });
    expect(view.revisions.fleet_work_version).toBe("7");
    expect(view.facts.intrinsic_urgency).toMatchObject({ value: "450", source: "fleet-policy", revision: "r1" });
    expect(view.facts.project).toMatchObject({ value: "Todo", source: "github_project_4" });
  });

  test("absent_dimension_stays_absent_not_zero", () => {
    const view = projectWorkView(base);
    expect(view.facts.importance).toEqual({ state: "absent", freshness: "absent" });
    expect(view.facts.intrinsic_urgency.state).toBe("absent");
    // Effective pressure is explicitly undefined until #9200 defines it.
    expect(view.facts.effective_pressure.state).toBe("not_yet_defined_by_9200");
  });

  test("stale_and_error_facts_are_not_zeroed", () => {
    const stale = projectWorkView({
      ...base,
      intrinsic_urgency: { value: "450", source: "fleet-policy", freshness: "stale" },
    });
    expect(stale.facts.intrinsic_urgency).toMatchObject({ value: "450", freshness: "stale" });
    expect(stale.freshness.state).toBe("stale");
    const err = projectWorkView({
      ...base,
      intrinsic_urgency: { value: "450", source: "fleet-policy", freshness: "error" },
    });
    expect(err.facts.intrinsic_urgency.state).toBe("error");
    expect(err.freshness.state).toBe("error");
  });

  test("equal_number_refs_under_different_repositories_do_not_merge", () => {
    const view = projectWorkView({
      ...base,
      source_refs: [
        { kind: "github_issue", repository_id: "111", resource_id: "42" },
        { kind: "github_issue", repository_id: "222", resource_id: "42" },
        { kind: "github_issue", repository_id: "111", resource_id: "42" },
      ],
    });
    expect(view.source_refs).toHaveLength(2);
  });

  test("hierarchy_and_hard_dependencies_stay_distinct", () => {
    const view = projectWorkView(
      { ...base, queue: { hard_blocker_refs: ["fleet://work/9"] } },
      { parent_refs: ["fleet://work/1"], advisory_refs: ["fleet://work/2"] },
    );
    expect(view.facts.relations.parent_refs).toEqual(["fleet://work/1"]);
    expect(view.facts.relations.hard_blocker_refs).toEqual(["fleet://work/9"]);
    expect(view.facts.relations.advisory_refs).toEqual(["fleet://work/2"]);
  });
});

describe("column projection matrix (oracle: design §3.1)", () => {
  test("done_requires_verified_terminal_receipt", () => {
    const noReceipt = projectWorkView({ ...base, github_state: { value: "closed", source: "github" } });
    expect(noReceipt.projection.column).not.toBe("Done");
    expect(noReceipt.projection.conflicts.map((c) => c.code)).toContain("completion_unverified");
    const receipt = projectWorkView({
      ...base,
      github_state: { value: "closed", source: "github" },
      completion: { verified_terminal_receipt: true },
    });
    expect(receipt.projection.column).toBe("Done");
  });

  test("unverified_terminal_signal_projects_review_per_mob02_r5", () => {
    // MOB-02 R5: terminal signal without a verified receipt is Review
    // with `completion_unverified`, never Done and never Backlog.
    const closed = projectWorkView({ ...base, github_state: { value: "closed", source: "github" } });
    expect(closed.projection.column).toBe("Review");
    expect(closed.projection.reason_codes).toContain("completion_unverified");
    const merged = projectWorkView({ ...base, github_state: { value: "merged", source: "github" } });
    expect(merged.projection.column).toBe("Review");
    expect(merged.projection.conflicts.map((c) => c.code)).toContain("completion_unverified");
  });

  test("parked_and_blocked_are_never_promoted_by_urgency", () => {
    const parked = projectWorkView({
      ...base,
      intrinsic_urgency: { value: "999", source: "fleet-policy" },
      queue: { eligible: true, high_or_above: true, parked: { reason: "waiting on vendor", checkpoint_ref: "cp-1" } },
    });
    expect(parked.projection.column).toBe("Parked");
    const blocked = projectWorkView({
      ...base,
      queue: { eligible: true, high_or_above: true, hard_blocker_refs: ["fleet://work/9"] },
    });
    expect(blocked.projection.column).toBe("Blocked");
    expect(blocked.facts.eligibility.state).toBe("not_eligible");
  });

  test("in_progress_requires_a_fresh_authoritative_claim", () => {
    const staleClaim = projectWorkView({ ...base, claim: { state: "claimed", worker: "lane-1", fresh: false } });
    expect(staleClaim.projection.column).not.toBe("In Progress");
    const freshClaim = projectWorkView({
      ...base,
      claim: { state: "claimed", worker: "lane-1", lane: "xaltha", fresh: true, session_binding: "verified" },
      session: { value: { provider: "opencode", state: "active" }, source: "session-adapter" },
    });
    expect(freshClaim.projection.column).toBe("In Progress");
    expect(freshClaim.facts.claim).toMatchObject({ worker: "lane-1" });
    expect(freshClaim.facts.session).toMatchObject({ value: "opencode:active" });
  });

  test("unverified_session_binding_is_a_conflict_not_a_live_session", () => {
    const view = projectWorkView({
      ...base,
      claim: { state: "claimed", worker: "lane-1", fresh: true, session_binding: "unknown" },
      session: { value: { provider: "slack", state: "posted" }, source: "session-adapter" },
    });
    expect(view.facts.session.state).toBe("binding_unknown");
    expect(view.projection.column).not.toBe("In Progress");
    expect(view.projection.conflicts.map((c) => c.code)).toContain("claimed_binding_unknown");
  });

  test("in_progress_requires_a_fresh_nonterminal_session_readback", () => {
    const claim = { state: "claimed", worker: "lane-1", fresh: true, session_binding: "verified" } as const;
    const cases: WorkProjectionInput[] = [
      { ...base, claim },
      { ...base, claim, session: { value: { provider: "opencode", state: "active" }, source: "session-adapter", freshness: "stale" } },
      { ...base, claim, session: { value: { provider: "opencode", state: "terminal" }, source: "session-adapter" } },
    ];
    for (const input of cases) expect(projectWorkView(input).projection.column).not.toBe("In Progress");
  });

  test("ready_requires_eligible_and_high_or_prepared_or_owner_assigned", () => {
    const ready = projectWorkView({ ...base, queue: { eligible: true, high_or_above: true } });
    expect(ready.projection.column).toBe("Ready");
    const merelyEligible = projectWorkView({ ...base, queue: { eligible: true } });
    expect(merelyEligible.projection.column).toBe("Backlog");
    const prepared = projectWorkView({ ...base, queue: { eligible: true, explicitly_prepared: true } });
    expect(prepared.projection.column).toBe("Ready");
  });

  test("manual_only_and_do_now_do_not_bypass_hard_gates", () => {
    const manual = projectWorkView({
      ...base,
      dispatch_directive: { value: "do_now", source: "fleet" },
      queue: { eligible: true, high_or_above: true, manual_only: true },
    });
    expect(manual.projection.column).toBe("Backlog");
    expect(manual.facts.eligibility.state).toBe("manual_only");
    // The directive itself is preserved as a fact, not applied as a bypass.
    expect(manual.facts.dispatch_directive).toMatchObject({ value: "do_now" });
  });

  test("projection_is_deterministic", () => {
    const input: WorkProjectionInput = {
      ...base,
      queue: { eligible: true, high_or_above: true, hard_blocker_refs: ["b", "a"] },
      source_refs: [
        { kind: "github_pr", repository_id: "9", resource_id: "2" },
        { kind: "github_issue", repository_id: "9", resource_id: "1" },
      ],
    };
    expect(projectWorkView(input)).toEqual(projectWorkView(input));
    expect(projectWorkView(input).facts.relations.hard_blocker_refs).toEqual(["a", "b"]);
  });
});
