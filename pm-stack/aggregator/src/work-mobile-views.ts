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
// - `recent` / `cold_backlog` use the `github_updated_at` fact MOB-01
//   carries (F1 landed): rows with a usable timestamp are in or out,
//   rows without one are `undecided`, and the view is `unavailable`
//   only when no row carries the dimension at all.
// - `needs_triage` needs issue labels, which `WorkView` does not carry:
//   `unavailable` until MOB-01 carries them (follow-up F2).
// - `triage` needs Area/Horizon beside Project Status; `WorkView`
//   carries neither, so a decidable-status row is still `undecided`
//   with `policy_undefined` (follow-up F3).
// - `blocking` / `critical_path` wait on MOB-08 relations (record gate
//   4): every row is `undecided` with `policy_undefined` (F4).
// - D5 orders by column, then `github_updated_at` descending (F1),
//   then `(authority_id, work_id)`; rows without a usable timestamp
//   sort after dated rows in the same column.
// - `groupWorkViews` (by `repo` / `project` / `owner`) and
//   `matchesSavedFilters` (record D1, D4, D6) are slice 2 in this
//   same module below. `repo` groups by the primary source ref's
//   `repository_id`, `project` by the project fact value (absent rows
//   form "No repo" / "No project" groups); `owner` stays a single
//   "Unknown author" bucket until MOB-01 carries an author fact (F5).

// V1 deviations from the record, all named so a later slice can lift
// them (see the record section 9 gates and section 10 non-goals):
// - The record names the input `PmWorkView`; that type does not exist.
//   The input here is the actual MOB-01 export `WorkView`.
// - `recent` / `cold_backlog` use the `github_updated_at` fact MOB-01
//   carries (F1 landed): rows with a usable timestamp are in or out,
//   rows without one are `undecided`, and the view is `unavailable`
//   only when no row carries the dimension at all.
// - `needs_triage` needs issue labels, which `WorkView` does not carry:
//   `unavailable` until MOB-01 carries them (follow-up F2).
// - `triage` needs Area/Horizon beside Project Status; `WorkView`
//   carries neither, so a decidable-status row is still `undecided`
//   with `policy_undefined` (follow-up F3).
// - `blocking` / `critical_path` wait on MOB-08 relations (record gate
//   4): every row is `undecided` with `policy_undefined` (F4).
// - D5 orders by column, then `github_updated_at` descending (F1),
//   then `(authority_id, work_id)`; rows without a usable timestamp
//   sort after dated rows in the same column.
// - `groupWorkViews` by `repo`/`project` and `matchesSavedFilters`
//   are slice 2 below. `owner` grouping stays a single "Unknown author"
//   bucket: `WorkView` carries no author fact (follow-up F5; the
//   saved-view `owner` dimension means the GitHub author on main).

import type { BoardColumn, WorkView } from "./work-projection.ts";
import { itemMatchesView, type ViewFilters, type ViewItemLike } from "./views.ts";

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

/** F1: read the carried github_updated_at as epoch ms, or say why it is unusable. */
const updatedAtOf = (view: WorkView): { ok: true; ms: number } | { ok: false; state: UndecidedState } => {
  const fact = view.facts.github_updated_at;
  if (!fact || !("value" in fact)) {
    const st = factState(fact);
    return { ok: false, state: (st === "present" ? "absent" : st) as UndecidedState };
  }
  const st = factState(fact);
  if (st !== "present") return { ok: false, state: st as UndecidedState };
  const ms = Date.parse(fact.value);
  if (Number.isNaN(ms)) return { ok: false, state: "invalid" };
  return { ok: true, ms };
};

/** D5: column order, then updated_at descending (F1), then key. Rows
 *  without a usable timestamp sort after dated rows in the same column. */
const orderRefs = (
  refs: WorkKey[],
  rank: Map<string, number>,
  recency: Map<string, number>,
): void => {
  refs.sort((a, b) => {
    const ka = a.authority_id + "|" + a.work_id;
    const kb = b.authority_id + "|" + b.work_id;
    const ra = rank.get(ka) ?? 8;
    const rb = rank.get(kb) ?? 8;
    if (ra !== rb) return ra - rb;
    const ta = recency.get(ka) ?? Number.NEGATIVE_INFINITY;
    const tb = recency.get(kb) ?? Number.NEGATIVE_INFINITY;
    if (ta !== tb) return tb - ta;
    return compareKeys(a, b);
  });
};

const recencyOf = (views: readonly WorkView[]): Map<string, number> => {
  const recency = new Map<string, number>();
  for (const view of views) {
    const ts = updatedAtOf(view);
    recency.set(keyOf(view), ts.ok ? ts.ms : Number.NEGATIVE_INFINITY);
  }
  return recency;
};

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
  // F1 landed: recent/cold_backlog are decided per row below from the
  // carried github_updated_at fact. The view is unavailable only when no
  // row carries the dimension at all (D2 pattern, as for claims).

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

  // F1: recent/cold_backlog need at least one row with a usable
  // timestamp; otherwise the page must say the dimension is not
  // connected, never render an empty list.
  if (id === "recent" || id === "cold_backlog") {
    const anyTs = known.some((v) => updatedAtOf(v).ok);
    if (!anyTs) {
      return { id, refs, undecided, unavailable: { reason: "fact_not_sourced", fact: "github.updated_at" }, freshness: "absent" };
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
      case "recent":
      case "cold_backlog": {
        // F1: decidable now that MOB-01 carries github_updated_at. A row
        // whose timestamp is missing or unusable is undecided (D3); a
        // row with a usable timestamp is in or out, never undecided.
        const ts = updatedAtOf(view);
        if (!ts.ok) {
          hold("github.updated_at", ts.state);
          break;
        }
        const nowMs = Date.parse(ctx.now);
        if (id === "recent") {
          if (nowMs - ts.ms <= Number(ctx.recent_window_ms)) refs.push(ref);
        } else if (column === "Backlog" && nowMs - ts.ms > Number(ctx.cold_after_ms)) {
          refs.push(ref);
        }
        break;
      }
      case "needs_triage":
        // Labels are still not carried by pm_work_view/v1 (F2).
        break;
    }
  }

  // D5: column order, then updated_at descending (F1), then key.
  const rank = new Map<string, number>();
  for (const view of known) {
    rank.set(keyOf(view), COLUMN_ORDER[view.projection.column] ?? 8);
  }
  const recency = recencyOf(known);
  orderRefs(refs, rank, recency);
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

// --- Slice 2: grouping and saved-filter reuse (record D1, D4, D6) ---
//
// `groupWorkViews` answers the "by repo / project / owner" control on
// `/work`: each group holds matched refs in D5 order, groups sort by
// label. `matchesSavedFilters` maps a `WorkView` onto the #1276
// `ViewItemLike` shape with the same field meanings as `ghItemLike`
// (record D6) and reuses `itemMatchesView`, so one matching rule
// serves both surfaces. No I/O, no clock, no module state.
//
// Truthfulness limits, all named:
// - `repo` groups by the primary `source_refs` entry's
//   `repository_id` (MOB-01 sorts `source_refs`, so [0] is stable).
//   Rows with no source ref form a "No repo" group, never merged.
// - `project` groups by the `project` fact value; rows with no
//   project fact form a "No project" group, never merged.
// - `owner` is a single "Unknown author" bucket: `WorkView` carries no
//   author fact, and the saved-view `owner` dimension means the GitHub
//   author on main (follow-up F5). A later slice lifts this once
//   MOB-01 carries the author; the bucket shape stays the same.
// - `area` / `horizon` / `epic` / `kind` are likewise absent from
//   `WorkView`: `workViewLike` maps them to `null`, so a saved filter
//   on one of those dimensions excludes the row (D6), never matches
//   it on a guessed value.

export type GroupBy = "repo" | "project" | "owner";

export interface WorkGroup {
  by: GroupBy;
  key: string;
  label: string;
  refs: WorkKey[];
}

const columnRankOf = (view: WorkView): number => COLUMN_ORDER[view.projection.column] ?? 8;

export function groupWorkViews(views: readonly WorkView[], by: GroupBy): WorkGroup[] {
  // Same input validation as `mobileView`: a repeated work key is a
  // validation error, never silently merged (record negative case).
  const seen = new Set<string>();
  for (const view of views) {
    const key = keyOf(view);
    if (seen.has(key)) {
      throw new Error(
        "work_mobile_views_duplicate_input: " + view.work.authority_id + "/" + view.work.work_id + " appears twice",
      );
    }
    seen.add(key);
  }

  const rank = new Map<string, number>();
  for (const view of views) {
    rank.set(keyOf(view), columnRankOf(view));
  }
  const recency = recencyOf(views);

  const buckets = new Map<string, WorkGroup>();
  for (const view of views) {
    const ref: WorkKey = { ...view.work };
    let key: string;
    let label: string;
    if (by === "repo") {
      const primary = view.source_refs[0];
      const repo = primary?.repository_id ?? null;
      key = "repo:" + (repo ?? "none");
      label = repo ?? "No repo";
    } else if (by === "project") {
      const project = view.facts.project;
      const value = "value" in project ? project.value : null;
      key = "project:" + (value ?? "none");
      label = value ?? "No project";
    } else {
      // F5: no author fact on `WorkView`; one bucket until MOB-01
      // carries it. The key/label shape is stable for that lift.
      key = "owner:unknown";
      label = "Unknown author";
    }
    const bucket = buckets.get(key);
    if (bucket) {
      bucket.refs.push(ref);
    } else {
      buckets.set(key, { by, key, label, refs: [ref] });
    }
  }

  const groups = [...buckets.values()];
  for (const group of groups) {
    orderRefs(group.refs, rank, recency);
  }
  groups.sort((a, b) => (a.label < b.label ? -1 : a.label > b.label ? 1 : a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
  return groups;
}

/** The #1276 `ViewItemLike` projection of a `WorkView`: the same
 *  field meanings as `ghItemLike`, with absent dimensions as `null`
 *  so filters on them exclude the row (record D6). */
export function workViewLike(view: WorkView): ViewItemLike {
  const primary = view.source_refs[0];
  const ref = primary
    ? primary.repository_id + "#" + primary.resource_id
    : view.work.authority_id + "/" + view.work.work_id;
  const github = view.facts.github_state;
  const project = view.facts.project;
  return {
    ref,
    repo: primary?.repository_id ?? null,
    area: null,
    horizon: null,
    // F5: no author fact on `WorkView` yet; never guess.
    owner: null,
    state: "value" in github ? github.value : null,
    epic: null,
    status: "value" in project ? project.value : null,
    kind: null,
  };
}

/** A saved `ViewFilters` set gives the same answer on a `WorkView`
 *  as `itemMatchesView` on its mapped item: one matching rule. */
export function matchesSavedFilters(view: WorkView, filters: ViewFilters): boolean {
  return itemMatchesView(workViewLike(view), filters);
}
