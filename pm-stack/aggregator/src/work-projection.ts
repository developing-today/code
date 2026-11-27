// Versioned canonical work read model and provenance (#9468, MOB-01 of
// the #9201 PM projection design, §3.1). One projection over the
// existing fleet-feed / PM Stack sources — no new database, no second
// Project writer, no policy in the client.
//
// Every fact in a `pm_work_view/v1` carries its source, revision and
// freshness, or an explicit absent / stale / error state. An absent
// dimension is absent: never zero, never guessed from a label, never
// synthesized from another score. Stale and errored inputs are carried
// as stale/error facts, not collapsed to zero or dropped.
//
// Identity is the shared `(authority_id, work_id)` pair plus stable
// provider source refs (kind + repository_id + resource_id). A GitHub
// `owner/repo#number` locator is display/input only: two refs with the
// same resource number under different repository ids are different
// work sources and never merge.
//
// The board column is a projection with an explanation (reason codes
// and conflicts), not an authoritative writable lifecycle enum. The
// underlying GitHub, queue, claim/lease, session, Project and dispatch
// facets stay separate in `facts`, so a lossy column mapping is always
// explainable from the view itself.

export const WORK_VIEW_SCHEMA_VERSION = "pm_work_view/v1";

export type FactFreshness = "live" | "stale" | "error" | "absent";

/** One sourced input dimension. Omit the whole field for "absent". */
export interface Sourced<T> {
  value: T;
  source: string;
  revision?: string;
  observed_at?: string;
  /** Defaults to "live". "stale"/"error" keep the value visible but
   *  mark the fact; they never rewrite it to zero. */
  freshness?: Exclude<FactFreshness, "absent">;
}

export interface SourceRef {
  kind: string;
  repository_id: string;
  resource_id: string;
}

export type Fact<T> =
  | {
      value: T;
      source: string;
      revision?: string;
      observed_at?: string;
      freshness: "live" | "stale";
    }
  | { state: "absent"; freshness: "absent" }
  | { state: "error"; source: string; revision?: string; freshness: "error" }
  | { state: string; freshness: "live" };

export interface ClaimInput {
  state: "unclaimed" | "claimed";
  worker?: string;
  lane?: string;
  /** A claim counts toward In Progress only when the authoritative
   *  claim read is fresh AND the adapter readback verifies the task/session
   *  binding. A label, queue row, dispatch intent or session shell alone
   *  is not proof of active execution. */
  fresh?: boolean;
  /** Session is displayed as bound only when readback confirms the
   *  task/session binding. */
  session_binding?: "verified" | "unknown";
}

export interface WorkProjectionInput {
  work: { authority_id: string; work_id: string };
  generated_at: string;
  source_refs?: SourceRef[];
  revisions?: {
    fleet_work_version?: string;
    github_revision?: string;
    project_revision?: string;
    activity_snapshot_ref?: string;
  };
  intrinsic_urgency?: Sourced<string>;
  importance?: Sourced<string>;
  /** Until #9200 defines effective pressure, an absent input projects
   *  the explicit `not_yet_defined_by_9200` state, not a number. */
  effective_pressure?: Sourced<string>;
  dispatch_directive?: Sourced<string>;
  workflow_state?: Sourced<string>;
  project_status?: Sourced<string>;
  github_state?: Sourced<"open" | "closed" | "merged">;
  /** Queue/eligibility facet, from the fleet-queue broker read. */
  queue?: {
    eligible?: boolean;
    hard_blocker_refs?: string[];
    manual_only?: boolean;
    /** Executable High-and-above under #9200. */
    high_or_above?: boolean;
    explicitly_prepared?: boolean;
    owner_assigned?: boolean;
    parked?: { reason: string; checkpoint_ref?: string } | null;
    blocked?: boolean;
  };
  claim?: ClaimInput;
  session?: Sourced<{ provider: string; state: string }> | { state: "none" };
  /** Done requires the designated completion policy's verified
   *  terminal receipt; a closed issue / merged PR / closed queue task
   *  alone never projects Done. */
  completion?: { verified_terminal_receipt: boolean };
  triage_required?: boolean;
}

export type BoardColumn =
  | "Inbox/Triage"
  | "Backlog"
  | "Ready"
  | "In Progress"
  | "Review"
  | "Blocked"
  | "Parked"
  | "Done";

export interface WorkView {
  schema_version: typeof WORK_VIEW_SCHEMA_VERSION;
  generated_at: string;
  work: { authority_id: string; work_id: string };
  source_refs: SourceRef[];
  revisions: NonNullable<WorkProjectionInput["revisions"]>;
  facts: {
    intrinsic_urgency: Fact<string>;
    importance: Fact<string>;
    effective_pressure: Fact<string>;
    dispatch_directive: Fact<string>;
    workflow_state: Fact<string>;
    project: Fact<string>;
    github_state: Fact<string>;
    eligibility: Fact<string> & { hard_blocker_refs: string[] };
    claim: Fact<string> & { worker?: string; lane?: string };
    session: Fact<string>;
    relations: {
      parent_refs: string[];
      hard_blocker_refs: string[];
      advisory_refs: string[];
    };
  };
  projection: {
    column: BoardColumn;
    reason_codes: string[];
    conflicts: { code: string; detail: string }[];
  };
  freshness: { state: FactFreshness; facets: Record<string, FactFreshness> };
}

function toFact<T>(input: Sourced<T> | undefined): Fact<T> {
  if (!input) return { state: "absent", freshness: "absent" };
  const freshness = input.freshness ?? "live";
  if (freshness === "error") {
    return { state: "error", source: input.source, revision: input.revision, freshness: "error" };
  }
  return {
    value: input.value,
    source: input.source,
    revision: input.revision,
    observed_at: input.observed_at,
    freshness,
  };
}

function factFreshness(fact: Fact<unknown>): FactFreshness {
  return fact.freshness;
}

const FRESHNESS_RANK: Record<FactFreshness, number> = { live: 0, absent: 1, stale: 2, error: 3 };

export function projectWorkView(
  input: WorkProjectionInput,
  relations: { parent_refs?: string[]; advisory_refs?: string[] } = {},
): WorkView {
  const hardBlockers = [...(input.queue?.hard_blocker_refs ?? [])].sort();
  const reasons = new Set<string>();
  const conflicts: WorkView["projection"]["conflicts"] = [];

  // Facts: provenance per fact, absent stays absent.
  const urgency = toFact(input.intrinsic_urgency);
  const importance = toFact(input.importance);
  const pressure: Fact<string> = input.effective_pressure
    ? toFact(input.effective_pressure)
    : { state: "not_yet_defined_by_9200", freshness: "live" };
  const directive = toFact(input.dispatch_directive);
  const workflow = toFact(input.workflow_state);
  const project = input.project_status
    ? (() => {
        const f = toFact(input.project_status);
        return "value" in f ? { ...f, value: input.project_status!.value } : f;
      })()
    : ({ state: "absent", freshness: "absent" } as Fact<string>);
  const github = toFact(input.github_state);

  const manualOnly = input.queue?.manual_only === true;
  const eligible = input.queue?.eligible === true && hardBlockers.length === 0;
  const eligibility: WorkView["facts"]["eligibility"] = {
    state: manualOnly ? "manual_only" : eligible ? "eligible" : "not_eligible",
    freshness: "live",
    hard_blocker_refs: hardBlockers,
  };

  const claimFresh = input.claim?.state === "claimed" && input.claim.fresh === true;
  const claim: WorkView["facts"]["claim"] =
    input.claim?.state === "claimed"
      ? {
          value: claimFresh ? "claimed" : "claimed_unverified",
          source: "fleet-queue",
          freshness: "live",
          worker: input.claim.worker,
          lane: input.claim.lane,
        }
      : { state: "unclaimed", freshness: "live" };

  let session: Fact<string>;
  if (!input.session || ("state" in input.session && input.session.state === "none" && !("value" in input.session))) {
    session = { state: "none", freshness: "live" };
  } else {
    const s = input.session as Sourced<{ provider: string; state: string }>;
    session =
      input.claim?.session_binding === "verified"
        ? toFact({ ...s, value: `${s.value.provider}:${s.value.state}` })
        : { state: "binding_unknown", freshness: "live" };
    if (input.claim?.session_binding !== "verified") {
      conflicts.push({
        code: "claimed_binding_unknown",
        detail: "session present but the task/session binding is not verified by readback",
      });
    }
  }

  // Column projection, in precedence order. Parked/blocked keep their
  // state and reason; urgency or owner assignment never promotes them.
  const parked = input.queue?.parked ?? null;
  const blocked = input.queue?.blocked === true || hardBlockers.length > 0;
  const verifiedDone = input.completion?.verified_terminal_receipt === true;
  const terminalSignal = ("value" in github && (github.value === "closed" || github.value === "merged")) || false;

  let column: BoardColumn;
  if (verifiedDone) {
    column = "Done";
    reasons.add("verified_terminal_receipt");
  } else if (parked) {
    column = "Parked";
    reasons.add("parked_with_reason");
  } else if (blocked) {
    column = "Blocked";
    reasons.add(hardBlockers.length > 0 ? "hard_blockers_present" : "queue_blocked");
  } else if (
    claimFresh &&
    input.claim?.session_binding === "verified" &&
    "value" in session &&
    session.freshness === "live" &&
    !session.value.endsWith(":terminal")
  ) {
    column = "In Progress";
    reasons.add("fresh_authoritative_claim");
    reasons.add("verified_session_binding");
  } else if (terminalSignal) {
    // Terminal signal without a verified receipt: conflict/pending,
    // never Done.
    column = "Backlog";
    reasons.add("completion_unverified");
    conflicts.push({
      code: "completion_unverified",
      detail: "closed/merged signal present but no verified terminal receipt from the completion policy",
    });
  } else if (
    eligible &&
    !manualOnly &&
    (input.queue?.high_or_above === true ||
      input.queue?.explicitly_prepared === true ||
      input.queue?.owner_assigned === true)
  ) {
    column = "Ready";
    reasons.add(
      input.queue?.high_or_above
        ? "eligible_high_or_above"
        : input.queue?.explicitly_prepared
          ? "explicitly_prepared"
          : "owner_assigned",
    );
  } else if (input.triage_required === true) {
    column = "Inbox/Triage";
    reasons.add("triage_required");
  } else {
    column = "Backlog";
    reasons.add(manualOnly ? "manual_only_excluded_from_automatic_claim" : "not_ready");
  }
  // do_now is a one-shot directive, never a bypass: it is preserved as
  // a fact above and deliberately does not change the column when the
  // item is manual_only, parked or blocked (handled by precedence).

  // Source refs: dedupe on the stable triple only; equal resource
  // numbers under different repository ids stay distinct.
  const refMap = new Map<string, SourceRef>();
  for (const r of input.source_refs ?? []) {
    refMap.set(`${r.kind} ${r.repository_id} ${r.resource_id}`, r);
  }
  const source_refs = [...refMap.values()].sort((a, b) =>
    `${a.kind}:${a.repository_id}:${a.resource_id}`.localeCompare(`${b.kind}:${b.repository_id}:${b.resource_id}`),
  );

  const facetFreshness: Record<string, FactFreshness> = {
    intrinsic_urgency: factFreshness(urgency),
    importance: factFreshness(importance),
    effective_pressure: factFreshness(pressure),
    dispatch_directive: factFreshness(directive),
    workflow_state: factFreshness(workflow),
    project: factFreshness(project),
    github_state: factFreshness(github),
  };
  const overall = Object.values(facetFreshness).reduce<FactFreshness>(
    (worst, f) => (FRESHNESS_RANK[f] > FRESHNESS_RANK[worst] ? f : worst),
    "live",
  );

  return {
    schema_version: WORK_VIEW_SCHEMA_VERSION,
    generated_at: input.generated_at,
    work: input.work,
    source_refs,
    revisions: input.revisions ?? {},
    facts: {
      intrinsic_urgency: urgency,
      importance,
      effective_pressure: pressure,
      dispatch_directive: directive,
      workflow_state: workflow,
      project,
      github_state: github,
      eligibility,
      claim,
      session,
      relations: {
        parent_refs: [...(relations.parent_refs ?? [])].sort(),
        hard_blocker_refs: hardBlockers,
        advisory_refs: [...(relations.advisory_refs ?? [])].sort(),
      },
    },
    projection: {
      column,
      reason_codes: [...reasons].sort(),
      conflicts: conflicts.sort((a, b) => a.code.localeCompare(b.code)),
    },
    freshness: { state: overall, facets: facetFreshness },
  };
}
