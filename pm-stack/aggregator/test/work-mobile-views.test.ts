// MOB-03 pure-filter tests, parts 1+2 (#9470 of the #9201 design).
// Oracle: the MOB-03 record D2/D3/D5 rules over the MOB-01 `WorkView`
// read model. V1 builds its `WorkView` inputs in code via
// `projectWorkView` (no JSON fixtures yet); a later slice adds
// `test/fixtures/work-mobile-views/expected.json` once `updated_at`,
// labels and relations land in the read model (follow-ups F1/F2/F4).
//
// Record tests 5 (`group_split`) and 6 (`saved_filters`) cover
// `groupWorkViews` / `matchesSavedFilters` in the second describe
// block below (slice 2). `repo` groups by the primary source ref's
// `repository_id`, `project` by the project fact value; `owner` stays
// a single "Unknown author" bucket until MOB-01 carries an author
// fact (F5).

import { describe, expect, test } from "bun:test";
import { projectWorkView, type WorkProjectionInput } from "../src/work-projection.ts";
import {
  groupWorkViews,
  matchesSavedFilters,
  mobileView,
  workViewLike,
  type ViewContext,
} from "../src/work-mobile-views.ts";
import { itemMatchesView } from "../src/views.ts";
// Type-only import keeps the runtime dependency one-way (views <- projection).
import type { WorkView } from "../src/work-projection.ts";

const GEN = "2026-11-26T12:00:00Z";
const CTX: ViewContext = {
  now: GEN,
  recent_window_ms: "86400000",
  cold_after_ms: "2592000000",
  viewer: { lane: "code-f" },
};

const base = (work_id: string, rest: Partial<WorkProjectionInput> = {}): WorkView =>
  projectWorkView({
    work: { authority_id: "fleet-authority", work_id },
    generated_at: GEN,
    ...rest,
  } as WorkProjectionInput);

const ready = (id: string): WorkView => base(id, { queue: { eligible: true, explicitly_prepared: true } });
const review = (id: string): WorkView => base(id, { github_state: { value: "closed", source: "github" } });
const blocked = (id: string): WorkView => base(id, { queue: { blocked: true } });
const backlog = (id: string): WorkView => base(id, {});
const inProgress = (id: string): WorkView =>
  base(id, {
    claim: { state: "claimed", fresh: true, worker: "w", lane: "code-f", session_binding: "verified" },
    session: { value: { provider: "p", state: "running" }, source: "adapter", observed_at: GEN },
  });

describe("work_mobile_views/v1 pure filter", () => {
  test("work_mobile_views_all_returns_refs_in_d5_order", () => {
    const views = [review("r1"), backlog("b1"), ready("q1"), inProgress("p1")];
    const got = mobileView(views, "all", CTX);
    expect(got.unavailable).toBeNull();
    // D5: Ready(1) < In Progress(2) < Review(3) < Backlog(6).
    expect(got.refs.map((r) => r.work_id)).toEqual(["q1", "p1", "r1", "b1"]);
  });

  test("work_mobile_views_ready_review_blocked_parked_filter_by_column", () => {
    const views = [ready("q1"), review("r1"), blocked("b1"), backlog("x1")];
    expect(mobileView(views, "ready", CTX).refs.map((r) => r.work_id)).toEqual(["q1"]);
    expect(mobileView(views, "review", CTX).refs.map((r) => r.work_id)).toEqual(["r1"]);
    expect(mobileView(views, "blocked_parked", CTX).refs.map((r) => r.work_id)).toEqual(["b1"]);
  });

  test("work_mobile_views_high_is_undecided_until_9200", () => {
    // Label-only urgency, however large, never places a row in `high` (D2).
    const views = [base("h1", { intrinsic_urgency: { value: "999", source: "fleet-policy" } }), ready("q1")];
    const got = mobileView(views, "high", CTX);
    expect(got.refs).toEqual([]);
    expect(got.unavailable).toBeNull();
    expect(got.undecided).toEqual([
      {
        ref: { authority_id: "fleet-authority", work_id: "h1" },
        deciding_fact: "intrinsic_urgency",
        state: "policy_undefined",
      },
      {
        ref: { authority_id: "fleet-authority", work_id: "q1" },
        deciding_fact: "intrinsic_urgency",
        state: "policy_undefined",
      },
    ]);
  });

  test("work_mobile_views_unsourced_claim_is_unavailable_not_empty", () => {
    // No row carries claim data: the page must say claims are not
    // connected, never render an empty "nothing assigned" list.
    const views = [backlog("b1")];
    for (const id of ["my_work", "active_agent"] as const) {
      const got = mobileView(views, id, CTX);
      expect(got.refs).toEqual([]);
      expect(got.unavailable).toEqual({ reason: "fact_not_sourced", fact: "claim" });
    }
    // Without a viewer the answer is the same, even with claim data.
    const withClaim = [inProgress("p1")];
    const noViewer: ViewContext = { ...CTX, viewer: null };
    expect(mobileView(withClaim, "my_work", noViewer).unavailable).toEqual({
      reason: "fact_not_sourced",
      fact: "claim",
    });
  });

  test("work_mobile_views_my_work_matches_viewer_lane_only", () => {
    const mine = inProgress("p1");
    const otherLane = base("p2", {
      claim: { state: "claimed", fresh: true, worker: "w", lane: "code-g", session_binding: "verified" },
      session: { value: { provider: "p", state: "running" }, source: "adapter", observed_at: GEN },
    });
    const got = mobileView([mine, otherLane], "my_work", CTX);
    expect(got.refs.map((r) => r.work_id)).toEqual(["p1"]);
    expect(got.undecided.map((u) => u.ref.work_id)).toEqual(["p2"]);
  });

  test("work_mobile_views_active_agent_needs_claim_and_session", () => {
    const both = inProgress("p1");
    const claimOnly = base("p2", {
      claim: { state: "claimed", fresh: true, worker: "w", lane: "code-f", session_binding: "unknown" },
      session: { state: "none" },
    });
    const got = mobileView([both, claimOnly], "active_agent", CTX);
    expect(got.refs.map((r) => r.work_id)).toEqual(["p1"]);
    expect(got.undecided.map((u) => u.ref.work_id)).toEqual(["p2"]);
  });

  test("work_mobile_views_count_undecided_rows", () => {
    // Absent deciding fact lands in `undecided` with fact + state (D3);
    // relations-gated views hold every row as `policy_undefined` (F4).
    const views = [backlog("b1")];
    const triage = mobileView(views, "triage", CTX);
    expect(triage.refs).toEqual([]);
    expect(triage.undecided).toEqual([
      { ref: { authority_id: "fleet-authority", work_id: "b1" }, deciding_fact: "project", state: "absent" },
    ]);
    const blocking = mobileView(views, "blocking", CTX);
    expect(blocking.refs).toEqual([]);
    expect(blocking.undecided).toEqual([
      {
        ref: { authority_id: "fleet-authority", work_id: "b1" },
        deciding_fact: "relations",
        state: "policy_undefined",
      },
    ]);
  });

  test("work_mobile_views_reject_unknown_schema_version", () => {
    const bad = { ...backlog("b1"), schema_version: "pm_work_view/v0" } as unknown as WorkView;
    const got = mobileView([bad, ready("q1")], "all", CTX);
    expect(got.refs.map((r) => r.work_id)).toEqual(["q1"]);
    expect(got.undecided).toEqual([
      { ref: { authority_id: "fleet-authority", work_id: "b1" }, deciding_fact: "schema_version", state: "invalid" },
    ]);
  });

  test("work_mobile_views_dedupe_is_a_validation_error", () => {
    const views = [backlog("b1"), ready("b1")];
    expect(() => mobileView(views, "all", CTX)).toThrow("work_mobile_views_duplicate_input");
    // Same work_id under a different authority is a different key: no merge.
    const other = projectWorkView({
      work: { authority_id: "other-authority", work_id: "b1" },
      generated_at: GEN,
    });
    const got = mobileView([backlog("b1"), other], "all", CTX);
    expect(got.refs).toHaveLength(2);
  });

  test("work_mobile_views_order_is_deterministic", () => {
    const views = [review("r1"), backlog("b1"), ready("q1"), blocked("k1"), inProgress("p1")];
    const a = mobileView(views, "all", CTX);
    const reversed = mobileView([...views].reverse(), "all", CTX);
    expect(reversed).toEqual(a);
    expect(JSON.stringify(reversed)).toBe(JSON.stringify(a));
  });

  test("work_mobile_views_missing_dimensions_are_unavailable", () => {
    const views = [ready("q1")];
    expect(mobileView(views, "needs_triage", CTX).unavailable).toEqual({
      reason: "fact_not_sourced",
      fact: "github.labels",
    });
    expect(mobileView(views, "recent", CTX).unavailable).toEqual({
      reason: "fact_not_sourced",
      fact: "github.updated_at",
    });
    expect(mobileView(views, "cold_backlog", CTX).unavailable).toEqual({
      reason: "fact_not_sourced",
      fact: "github.updated_at",
    });
  });
});

describe("work_mobile_views/v1 grouping and saved filters (slice 2)", () => {
  const withRefs = (id: string, repository_id: string, resource_id = "42"): WorkView =>
    base(id, { source_refs: [{ kind: "github_issue", repository_id, resource_id }] });

  test("work_mobile_views_group_split_on_ambiguous_identity", () => {
    // Same resource number under two repository ids: MOB-01 D3 keeps
    // them distinct, and `repo` grouping splits them (record test 5).
    const views = [withRefs("a1", "111"), withRefs("b1", "222")];
    const groups = groupWorkViews(views, "repo");
    expect(groups.map((g) => g.label)).toEqual(["111", "222"]);
    expect(groups.map((g) => g.refs.map((r) => r.work_id))).toEqual([["a1"], ["b1"]]);
    // Rows with no source ref form their own group, never merged.
    const mixed = [...views, backlog("c1")];
    const labels = groupWorkViews(mixed, "repo").map((g) => g.label);
    expect(labels).toContain("No repo");
  });

  test("work_mobile_views_group_by_project_and_owner", () => {
    const withStatus = base("s1", { project_status: { value: "In Progress", source: "project" } });
    const without = backlog("b1");
    const groups = groupWorkViews([withStatus, without], "project");
    expect(groups.map((g) => g.label).sort()).toEqual(["In Progress", "No project"]);
    // F5: no author fact on WorkView, so owner grouping is one named
    // bucket until MOB-01 carries it — never one bucket per row.
    const ownerGroups = groupWorkViews([withStatus, without], "owner");
    expect(ownerGroups).toHaveLength(1);
    expect(ownerGroups[0].label).toBe("Unknown author");
    expect(ownerGroups[0].refs.map((r) => r.work_id).sort()).toEqual(["b1", "s1"]);
  });

  test("work_mobile_views_saved_filters_reuse_item_matches_view", () => {
    // Record test 6: a ViewFilters set gives the same answer through
    // matchesSavedFilters as itemMatchesView on the mapped item.
    const view = withRefs("a1", "developing-today/code", "9469");
    const like = workViewLike(view);
    expect(like.ref).toBe("developing-today/code#9469");
    expect(like.repo).toBe("developing-today/code");
    const filters = { repo: "developing-today/code" } as const;
    expect(matchesSavedFilters(view, filters)).toBe(true);
    expect(itemMatchesView(like, filters)).toBe(matchesSavedFilters(view, filters));
    expect(matchesSavedFilters(view, { repo: "other/repo" })).toBe(false);
    // A filter on an absent dimension (owner/author, F5) excludes the
    // row instead of matching it on a guess (record D6).
    expect(matchesSavedFilters(view, { owner: "someone" })).toBe(false);
  });

  test("work_mobile_views_group_order_is_deterministic", () => {
    const views = [withRefs("a1", "zzz"), withRefs("b1", "aaa"), backlog("c1")];
    const a = groupWorkViews(views, "repo");
    const reversed = groupWorkViews([...views].reverse(), "repo");
    expect(reversed).toEqual(a);
    expect(JSON.stringify(reversed)).toBe(JSON.stringify(a));
  });

  test("work_mobile_views_group_dedupe_is_a_validation_error", () => {
    expect(() => groupWorkViews([backlog("b1"), ready("b1")], "repo")).toThrow("work_mobile_views_duplicate_input");
  });

  test("work_mobile_views_owner_groups_by_carried_author", () => {
    // MOB-03 D1/D4 (F5): owner grouping follows the carried GitHub
    // author fact; rows with no author fact form one named bucket,
    // never one bucket per row and never merged into an author.
    const alice = base("a1", { github_author: { value: "alice", source: "github" } });
    const bob = base("b1", { github_author: { value: "bob", source: "github" } });
    const unknown = backlog("c1");
    const groups = groupWorkViews([bob, unknown, alice], "owner");
    expect(groups.map((g) => g.label)).toEqual(["Unknown author", "alice", "bob"]);
    expect(groups.map((g) => g.refs.map((r) => r.work_id))).toEqual([["c1"], ["a1"], ["b1"]]);
  });

  test("work_mobile_views_saved_filters_see_carried_author", () => {
    // F5: the carried author maps onto the saved-view `owner`
    // dimension with the same meaning as the #1276 item field; an
    // absent dimension stays null so a filter on it excludes the row
    // (record D6), never matches it on a guess.
    const carried = base("a1", { github_author: { value: "alice", source: "github" } });
    expect(workViewLike(carried).owner).toBe("alice");
    expect(matchesSavedFilters(carried, { owner: "alice" })).toBe(true);
    expect(matchesSavedFilters(carried, { owner: "bob" })).toBe(false);
    expect(matchesSavedFilters(backlog("b1"), { owner: "alice" })).toBe(false);
  });
});
