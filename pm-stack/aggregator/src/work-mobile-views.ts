// Mobile queue, item detail and saved views, pure filter part 1
// (#9470, MOB-03 of the #9201 PM projection design). The design record is
// doc/2026-11-26T02-57-22Z_design_mob03_mobile_queue_detail_saved_views/.
//
// `mobileView` answers one mobile list over `WorkView[]` (the MOB-01
// `pm_work_view/v1` read model, `src/work-projection.ts`): matched refs
// in D5 order, undecidable rows counted with their deciding fact and
// state (D3, never quietly dropped), and a view-level `unavailable`
// when the deciding dimension is not sourced at all. No I/O, no clock,
// no module state: `ctx.now` and both windows are injected, durations
// are decimal strings, same input gives byte-identical output.
//
// V1 deviations from the record, all named so a later slice can lift
// them (see the record section 9 gates and section 10 non-goals):
// - The record names the input `PmWorkView`; that type does not exist.
//   The input here is the actual MOB-01 export `WorkView`.
// - `recent` / `cold_backlog` need the issue `updated_at`, which
//   `WorkView` does not carry: both views return `unavailable` with
//   `fact_not_sourced` until MOB-01 carries it (follow-up F1).
// - `needs_triage` needs issue labels, which `WorkView` does not carry:
//   `unavailable` until MOB-01 carries them (follow-up F2).
// - `triage` needs Area/Horizon beside Project Status; `WorkView`
//   carries neither, so a decidable-status row is still `undecided`
//   with `policy_undefined` (follow-up F3).
// - `blocking` / `critical_path` wait on MOB-08 relations (record gate
//   4): every row is `undecided` with `policy_undefined` (F4).
// - D5 orders by column, then `updated_at` descending; `updated_at` is
//   not in `WorkView`, so V1 orders by column, then
//   `(authority_id, work_id)` (F1 covers the missing key).
// - `groupWorkViews` and `matchesSavedFilters` (record D1, D4, D6) are
//   not in this slice: `WorkView` source refs carry no `owner/repo`
//   locator, the project fact carries no board id, and no author fact
//   exists, so grouping would be a single bucket. Slice 2 owns them.

import type { BoardColumn, WorkView } from "./work-projection.ts";

export type MobileViewId =
  | "all"
  | "triage"
  | "needs_triage"
  | "ready"
  | "high"
  | "critical_path"
  | "blocking"
  | "my_work"
  | "active_agent"
  | "review"
  | "blocked_parked"
  | "recent"
  | "cold_backlog";

export interface ViewerRef {
  lane: string;
}

export interface ViewContext {
  now: string;
  recent_window_ms: string;
  cold_after_ms: string;
  viewer: ViewerRef | null;
}

export interface WorkKey {
  authority_id: string;
  work_id: string;
}

export type MobileFactKey =
  | "project"
  | "github"
  | "github.labels"
  | "github.updated_at"
  | "intrinsic_urgency"
  | "claim"
  | "session"
  | "relations"
  | "projection.column"
  | "schema_version";

export type UndecidedState = "absent" | "invalid" | "error" | "stale" | "policy_undefined";

export interface UndecidedRow {
  ref: WorkKey;
  deciding_fact: MobileFactKey;
  state: UndecidedState;
}

export interface MobileViewResult {
  id: MobileViewId;
  /** Matched rows, in D5 order. */
  refs: WorkKey[];
  /** Rows that could not be placed, each naming why. */
  undecided: UndecidedRow[];
  /** The whole view cannot be computed; refs is then empty. */
  unavailable: null | { reason: "fact_not_sourced"; fact: MobileFactKey };
  freshness: "live" | "fixtures" | "stale" | "error" | "absent";
}

// D5 column order.
const COLUMN_ORDER: Record<BoardColumn, number> = {
  "Inbox/Triage": 0,
  Ready: 1,
  "In Progress": 2,
  Review: 3,
  Blocked: 4,
  Parked: 5,
  Backlog: 6,
  Done: 7,
};

const FRESHNESS_RANK = { live: 0, fixtures: 1, absent: 2, stale: 3, error: 4 } as const;

type FactState = "present" | "absent" | "invalid" | "error" | "stale";

const factState = (fact: { freshness?: string; state?: string } | undefined): FactState => {
  if (!fact) return "absent";
  if (fact.state === "absent") return "absent";
  if (fact.state !== undefined && fact.state !== null && fact.state !== "present") {
    // MOB-01 present facts carry no `state` marker; named non-present
    // states (unclaimed, eligible, binding_unknown, ...) mean the fact
    // was read. Only treat real problem markers as non-present.
    const s = fact.state;
    if (s === "error") return "error";
    if (s === "stale") return "stale";
    return "present";
  }
  if (fact.freshness === "error") return "error";
  if (fact.freshness === "stale") return "stale";
  return "present";
};

const keyOf = (w: { work: WorkKey }): string => w.work.authority_id + "|" + w.work.work_id;

const compareKeys = (a: WorkKey, b: WorkKey): number =>
  a.authority_id < b.authority_id
    ? -1
    : a.authority_id > b.authority_id
      ? 1
      : a.work_id < b.work_id
        ? -1
        : a.work_id > b.work_id
          ? 1
          : 0;

export function mobileView(
  views: readonly WorkView[],
  id: MobileViewId,
  ctx: ViewContext,
): MobileViewResult {
  const refs: WorkKey[] = [];
  const undecided: UndecidedRow[] = [];
  const unavailable = (fact: MobileFactKey): MobileViewResult => ({
    id,
    refs: [],
    undecided: [],
    unavailable: { reason: "fact_not_sourced", fact },
    freshness: "absent",
  });

  // Input validation first: unknown schema versions are excluded and
  // counted (record test 8); a repeated work key is a validation
  // error, never silently merged (record test 9, negative duplicate).
  const known: WorkView[] = [];
  const seen = new Set<string>();
  for (const view of views) {
    if (view.schema_version !== "pm_work_view/v1") {
      undecided.push({ ref: { ...view.work }, deciding_fact: "schema_version", state: "invalid" });
      continue;
    }
    const key = keyOf(view);
    if (seen.has(key)) {
      throw new Error(
        "work_mobile_views_duplicate_input: " + view.work.authority_id + "/" + view.work.work_id + " appears twice",
      );
    }
    seen.add(key);
    known.push(view);
  }

  // Views whose deciding dimension is absent from the V1 read model.
  if (id === "needs_triage") return { ...unavailable("github.labels"), undecided };
  if (id === "recent" || id === "cold_backlog") return { ...unavailable("github.updated_at"), undecided };

  // Claim-gated views need a viewer, and need at least one row that
  // carries live claim data; otherwise the page must say claims are
  // not connected, never "nothing assigned to you" (record D2 notes).
  if (id === "my_work" || id === "active_agent") {
    if (ctx.viewer === null) {
      return { id, refs, undecided, unavailable: { reason: "fact_not_sourced", fact: "claim" }, freshness: "absent" };
    }
    const anyClaim = known.some((v) => "value" in v.facts.claim);
    if (!anyClaim) {
      return { id, refs, undecided, unavailable: { reason: "fact_not_sourced", fact: "claim" }, freshness: "absent" };
    }
  }

  for (const view of known) {
    const ref: WorkKey = { ...view.work };
    const hold = (deciding_fact: MobileFactKey, state: UndecidedState): void => {
      undecided.push({ ref, deciding_fact, state });
    };
    const column = view.projection.column;
    switch (id) {
      case "all":
        refs.push(ref);
        break;
      case "ready":
      case "review":
      case "blocked_parked": {
        const want: BoardColumn[] =
          id === "ready" ? ["Ready"] : id === "review" ? ["Review"] : ["Blocked", "Parked"];
        if (want.includes(column)) {
          refs.push(ref);
        }
        break;
      }
      case "high":
        // Record D2: until #9200 exports its predicate, urgency —
        // however spelled — never places a row in `high`.
        hold("intrinsic_urgency", "policy_undefined");
        break;
      case "triage": {
        const st = factState(view.facts.project);
        if (st !== "present") {
          hold("project", st);
        } else {
          // Project Status is decidable here, but Area/Horizon are not
          // carried by pm_work_view/v1, so V1 cannot confirm the D2
          // predicate (follow-up F3).
          hold("project", "policy_undefined");
        }
        break;
      }
      case "blocking":
      case "critical_path":
        // Record gate 4: MOB-08 relations are not on main yet.
        hold("relations", "policy_undefined");
        break;
      case "my_work": {
        const claim = view.facts.claim;
        const session = view.facts.session;
        if ("value" in claim && claim.value === "claimed" && claim.lane === ctx.viewer?.lane) {
          refs.push(ref);
        } else if (!("value" in claim)) {
          hold("claim", factState(claim) as UndecidedState);
        } else if ("value" in session) {
          // A verified-bound session exists but the claim does not tie
          // it to this viewer: placement needs a rule V1 does not have.
          hold("session", "policy_undefined");
        }
        break;
      }
      case "active_agent": {
        const claim = view.facts.claim;
        const session = view.facts.session;
        const claimedLive = "value" in claim && claim.value === "claimed";
        const boundVerified = "value" in session;
        if (claimedLive && boundVerified) {
          refs.push(ref);
        } else if (!("value" in claim)) {
          hold("claim", factState(claim) as UndecidedState);
        } else if (!boundVerified) {
          const st = factState(session);
          hold("session", st === "present" ? "policy_undefined" : (st as UndecidedState));
        }
        break;
      }
      case "needs_triage":
      case "recent":
      case "cold_backlog":
        // Handled by the view-level `unavailable` above.
        break;
    }
  }

  // D5: column order, then (authority_id, work_id).
  const rank = new Map<string, number>();
  for (const view of known) {
    rank.set(keyOf(view), COLUMN_ORDER[view.projection.column] ?? 8);
  }
  refs.sort((a, b) => {
    const ra = rank.get(a.authority_id + "|" + a.work_id) ?? 8;
    const rb = rank.get(b.authority_id + "|" + b.work_id) ?? 8;
    return ra - rb || compareKeys(a, b);
  });
  undecided.sort((a, b) => compareKeys(a.ref, b.ref));

  // Freshness is the worst facet state over placed and unplaced rows
  // (MOB-01 D7 pattern, as in `projectWorkView`).
  let freshness: MobileViewResult["freshness"] = "live";
  for (const view of known) {
    const state = view.freshness.state;
    const rankNum = FRESHNESS_RANK[state as keyof typeof FRESHNESS_RANK] ?? 0;
    if (rankNum > FRESHNESS_RANK[freshness]) {
      freshness = state;
    }
  }
  if (known.length === 0) freshness = "absent";

  return { id, refs, undecided, unavailable: null, freshness };
}
