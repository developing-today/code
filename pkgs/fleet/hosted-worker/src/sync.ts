// Sync door engine (#12591; design §7.2, the hosted form of
// #9016's sync-tool scope). Two verb families, one op vocabulary
// shared with the native doors:
//
//   Project sync — the writes project-reconcile.sh computes, as
//     typed ops executed server-side over the fleet App rotation:
//       { "op": "add-issue",  repo, number, kind?, status?, content_id? }
//       { "op": "set-status", item_id? | issue, field?, to }
//     These are the script's own outbox op names and fields (its
//     action-outbox entries are {op:"set-status", item_id, issue,
//     field, from, to} and {op:"add-issue", repo, number, status,
//     kind, content_id}), with the resolutions the script does by
//     hand — project id, field id, option id, item id, content
//     id — done server-side from one state read. The endpoint
//     retires, for hosted callers, the script's per-call
//     `gh project item-add` / `gh project item-edit` invocations
//     and the outbox file shape they fall back to.
//
//   Queue/GitHub sync — the recurring scripted flows (program
//     tending, stewardship verdict posting, pool reviews) as one
//     endpoint: a named flow posts one marked note (comment) plus
//     optional label edits on one issue/PR. The flows today
//     compose raw `gh issue comment` / `gh pr comment` /
//     `gh issue edit --add-label` calls in job bodies; the
//     endpoint retires those compositions for hosted callers.
//
// Retry-safety (inherited from the native design, #8887/gh.rs):
// a write whose outcome is unknown is never replayed blindly —
// the request may have landed, and repeating it could
// double-post. The door therefore executes ops SERIALLY (never
// one batched mutation), checks each op's already-applied state
// before writing (project membership, current field value,
// marker presence), and reports per-op outcomes, so a caller
// retrying a whole request after a mid-flow failure re-runs
// safely: landed ops answer already_* and only the remainder
// writes. Rate-limit failover itself happens inside the rotation
// (src/github.ts), under the same rule as every hosted write.
//
// Errors are value-free: refusals name the rule and the field,
// never a token or a provider response body.

import type { AuthDeps } from "./auth.ts";
import { FailClosed } from "./errors.ts";
import type { GitHubCommentSurface, GitHubProjectState, GitHubProjectSurface, ProjectItemState } from "./github.ts";

/** The most ops one project-sync request carries. The script
 * applies one pass's proposals per run and bounds its own write
 * count; 50 is a stated bound in the same spirit — a caller with
 * more work sends more requests, each independently retry-safe. */
export const MAX_SYNC_OPS = 50;

/** Raw-gh call sites the project endpoint retires for hosted
 * callers (acceptance: each endpoint names them). */
export const PROJECT_SYNC_RETIRES: string[] = [
  "gh project item-add (project-reconcile.sh's add path)",
  "gh project item-edit (project-reconcile.sh's status-set path)",
  "gh project field-list / item-list resolution reads the script performs per call",
  "the action-outbox project-sync file shape and its project-sync-apply.sh drain (for hosted callers)",
];

/** Raw-gh call sites the queue/GitHub endpoint retires for
 * hosted callers: the comment/label compositions in the three
 * flows' job bodies. */
export const FLOW_SYNC_RETIRES: string[] = [
  "gh issue comment / gh pr comment (stewardship verdict posting)",
  "gh issue comment (program tending notes)",
  "gh issue comment (pool review notes)",
  "gh issue edit --add-label / --remove-label (label flips in those job bodies)",
];

// --- Dependencies -------------------------------------------------

export interface ProjectSyncDeps {
  projects: GitHubProjectSurface;
}

export interface FlowSyncDeps {
  comments: GitHubCommentSurface;
}

export function projectSyncDepsFrom(deps: AuthDeps): ProjectSyncDeps {
  if (!deps.githubProjects) {
    throw new FailClosed(
      503,
      "sync_not_configured",
      "this deployment has no GitHub project surface configured; failing closed",
    );
  }
  return { projects: deps.githubProjects };
}

export function flowSyncDepsFrom(deps: AuthDeps): FlowSyncDeps {
  if (!deps.githubComments) {
    throw new FailClosed(
      503,
      "sync_not_configured",
      "this deployment has no GitHub comment surface configured; failing closed",
    );
  }
  return { comments: deps.githubComments };
}

// --- Shared body-parsing helpers (the doors' idiom) ---------------

const REPO_PATTERN = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;

function bodyObject(raw: unknown): Record<string, unknown> {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    throw new FailClosed(400, "invalid_body", "request body must be a JSON object");
  }
  return raw as Record<string, unknown>;
}

function requiredString(body: Record<string, unknown>, field: string): string {
  const value = body[field];
  if (typeof value !== "string" || value.trim() === "") {
    throw new FailClosed(400, "invalid_body", `${field} must be a non-empty string`);
  }
  return value;
}

function optionalString(body: Record<string, unknown>, field: string): string | null {
  const value = body[field];
  if (value === undefined || value === null) return null;
  if (typeof value !== "string" || value.trim() === "") {
    throw new FailClosed(400, "invalid_body", `${field} must be a non-empty string when present`);
  }
  return value;
}

function requiredRepo(body: Record<string, unknown>, field: string): string {
  const repo = requiredString(body, field);
  if (!REPO_PATTERN.test(repo)) {
    throw new FailClosed(400, "invalid_body", `${field} must be in owner/name form`);
  }
  return repo;
}

function requiredPositiveInt(body: Record<string, unknown>, field: string): number {
  const value = body[field];
  if (typeof value !== "number" || !Number.isInteger(value) || value <= 0) {
    throw new FailClosed(400, "invalid_body", `${field} must be a positive integer`);
  }
  return value;
}

function optionalKind(body: Record<string, unknown>): "issue" | "pr" {
  const raw = body["kind"];
  if (raw === undefined || raw === null) return "issue";
  if (raw !== "issue" && raw !== "pr") {
    throw new FailClosed(400, "invalid_body", 'kind must be "issue" or "pr"');
  }
  return raw;
}

// --- Project sync -------------------------------------------------

export interface ProjectRef {
  owner: string;
  number: number;
}

export type ProjectSyncOp =
  | {
      op: "add-issue";
      repo: string;
      number: number;
      kind: "issue" | "pr";
      status: string | null;
      contentId: string | null;
    }
  | {
      op: "set-status";
      itemId: string | null;
      issue: { repo: string; number: number } | null;
      field: string;
      to: string;
    };

export interface ProjectSyncRequest {
  project: ProjectRef;
  ops: ProjectSyncOp[];
}

/** "owner/repo#123" — the issue reference the script's outbox
 * entries carry for a set-status op. */
function parseIssueRef(raw: string): { repo: string; number: number } {
  const match = /^([A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+)#(\d+)$/.exec(raw);
  if (!match) {
    throw new FailClosed(400, "invalid_body", `issue reference ${raw} is not in owner/name#number form`);
  }
  return { repo: match[1], number: Number(match[2]) };
}

function parseProjectOp(raw: unknown, index: number): ProjectSyncOp {
  const body = bodyObject(raw);
  const opName = body["op"];
  if (opName === "add-issue") {
    return {
      op: "add-issue",
      repo: requiredRepo(body, "repo"),
      number: requiredPositiveInt(body, "number"),
      kind: optionalKind(body),
      status: optionalString(body, "status"),
      contentId: optionalString(body, "content_id"),
    };
  }
  if (opName === "set-status") {
    const itemId = optionalString(body, "item_id");
    const issueRaw = optionalString(body, "issue");
    if (itemId === null && issueRaw === null) {
      throw new FailClosed(400, "invalid_body", `ops[${index}]: a set-status op names its item by item_id or by issue`);
    }
    return {
      op: "set-status",
      itemId,
      issue: issueRaw === null ? null : parseIssueRef(issueRaw),
      field: optionalString(body, "field") ?? "Status",
      to: requiredString(body, "to"),
    };
  }
  throw new FailClosed(
    400,
    "invalid_body",
    `ops[${index}]: unknown sync op ${typeof opName === "string" ? opName : "(missing)"} — the vocabulary is add-issue, set-status`,
  );
}

export function parseProjectSyncRequest(raw: unknown): ProjectSyncRequest {
  const body = bodyObject(raw);
  const projectRaw = body["project"];
  if (typeof projectRaw !== "object" || projectRaw === null || Array.isArray(projectRaw)) {
    throw new FailClosed(400, "invalid_body", "project must be an object naming owner and number");
  }
  const projectBody = projectRaw as Record<string, unknown>;
  const owner = requiredString(projectBody, "owner");
  const number = requiredPositiveInt(projectBody, "number");
  const opsRaw = body["ops"];
  if (!Array.isArray(opsRaw) || opsRaw.length === 0) {
    throw new FailClosed(400, "invalid_body", "ops must be a non-empty list of sync ops");
  }
  if (opsRaw.length > MAX_SYNC_OPS) {
    throw new FailClosed(
      400,
      "invalid_body",
      `ops carries ${opsRaw.length} ops; one request carries at most ${MAX_SYNC_OPS}`,
    );
  }
  return {
    project: { owner, number },
    ops: opsRaw.map((op, i) => parseProjectOp(op, i)),
  };
}

export interface ProjectOpResult {
  op: string;
  /** added | already_present | status_set | already_set | failed */
  outcome: string;
  item_id?: string;
  repo?: string;
  number?: number;
  field?: string;
  from?: string | null;
  to?: string;
  /** For outcome "failed": the refusal's machine code. */
  error?: string;
}

export interface ProjectSyncResponse {
  project: { owner: string; number: number; id: string };
  binding_id: string;
  results: ProjectOpResult[];
  counts: { applied: number; already: number; failed: number };
  retires: string[];
}

/** Set one item's field to a named option against the tracked
 * snapshot; shared by the add path (the script's add_and_set
 * sets the initial status after an add) and the set-status op.
 * Returns the op outcome; throws nothing — a surface refusal is
 * recorded as the result's failure, per-op, like the script's
 * per-write APPLIED / FAILED lines. */
async function applyStatus(
  deps: ProjectSyncDeps,
  state: GitHubProjectState,
  item: ProjectItemState,
  fieldName: string,
  to: string,
  result: ProjectOpResult,
): Promise<void> {
  const field = state.fields.find((f) => f.name === fieldName);
  if (!field) {
    result.outcome = "failed";
    result.error = "field_not_found";
    return;
  }
  const option = field.options.find((o) => o.name === to);
  if (!option) {
    result.outcome = "failed";
    result.error = "option_not_found";
    return;
  }
  const from = item.values[fieldName] ?? null;
  result.field = fieldName;
  result.from = from;
  result.to = to;
  if (from === to) {
    result.outcome = result.outcome === "added" ? "added" : "already_set";
    return;
  }
  try {
    await deps.projects.setProjectItemSingleSelect(state.projectId, item.itemId, field.id, option.id);
  } catch (e) {
    result.outcome = "failed";
    result.error = e instanceof FailClosed ? e.code : "github_project_failed";
    return;
  }
  item.values[fieldName] = to;
  if (result.outcome !== "added") result.outcome = "status_set";
}

export async function runProjectSync(
  deps: ProjectSyncDeps,
  bindingId: string,
  request: ProjectSyncRequest,
): Promise<ProjectSyncResponse> {
  const state = await deps.projects.getProjectState(request.project.owner, request.project.number);
  if (!state) {
    throw new FailClosed(
      404,
      "project_not_visible",
      `no project ${request.project.owner}#${request.project.number} is visible to this deployment's GitHub rotation`,
    );
  }
  const results: ProjectOpResult[] = [];
  for (const op of request.ops) {
    if (op.op === "add-issue") {
      const result: ProjectOpResult = {
        op: op.op,
        outcome: "failed",
        repo: op.repo,
        number: op.number,
      };
      const existing = state.items.find((i) => i.repo === op.repo && i.kind === op.kind && i.number === op.number);
      if (existing) {
        result.outcome = "already_present";
        result.item_id = existing.itemId;
        if (op.status !== null) {
          await applyStatus(deps, state, existing, "Status", op.status, result);
          // A landed status write reports as status_set (a write
          // applied); only the no-write case stays already_*.
          if (result.outcome === "already_set") result.outcome = "already_present";
        }
      } else {
        const contentId = op.contentId ?? (await deps.projects.resolveContentId(op.repo, op.kind, op.number));
        if (contentId === null) {
          result.error = "content_not_found";
          results.push(result);
          continue;
        }
        try {
          const { itemId } = await deps.projects.addProjectItem(state.projectId, contentId);
          const item: ProjectItemState = {
            itemId,
            repo: op.repo,
            kind: op.kind,
            number: op.number,
            values: {},
          };
          state.items.push(item);
          result.outcome = "added";
          result.item_id = itemId;
          if (op.status !== null) {
            await applyStatus(deps, state, item, "Status", op.status, result);
          }
        } catch (e) {
          result.error = e instanceof FailClosed ? e.code : "github_project_failed";
        }
      }
      results.push(result);
      continue;
    }
    // set-status
    const result: ProjectOpResult = { op: op.op, outcome: "failed" };
    const item =
      op.itemId !== null
        ? state.items.find((i) => i.itemId === op.itemId)
        : state.items.find((i) => op.issue !== null && i.repo === op.issue.repo && i.number === op.issue.number);
    if (!item) {
      result.error = "item_not_in_project";
      results.push(result);
      continue;
    }
    result.item_id = item.itemId;
    result.repo = item.repo;
    result.number = item.number;
    await applyStatus(deps, state, item, op.field, op.to, result);
    results.push(result);
  }
  const counts = { applied: 0, already: 0, failed: 0 };
  for (const r of results) {
    if (r.outcome === "failed") counts.failed += 1;
    else if (r.outcome.startsWith("already")) counts.already += 1;
    else counts.applied += 1;
  }
  return {
    project: { owner: request.project.owner, number: request.project.number, id: state.projectId },
    binding_id: bindingId,
    results,
    counts,
    retires: PROJECT_SYNC_RETIRES,
  };
}

// --- Queue/GitHub sync flows --------------------------------------

export const SYNC_FLOWS = ["verdict", "tend", "review"] as const;
export type SyncFlow = (typeof SYNC_FLOWS)[number];

export interface FlowSyncRequest {
  flow: SyncFlow;
  repo: string;
  number: number;
  kind: "issue" | "pr";
  marker: string;
  body: string;
  labels: { add: string[]; remove: string[] } | null;
}

/** A flow marker lands inside an HTML comment in the posted
 * note, so it is a bounded token charset — no whitespace, no
 * comment-breaking characters. */
const MARKER_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._/-]{0,127}$/;

export function parseFlowSyncRequest(raw: unknown): FlowSyncRequest {
  const body = bodyObject(raw);
  const flowRaw = body["flow"];
  if (typeof flowRaw !== "string" || !(SYNC_FLOWS as readonly string[]).includes(flowRaw)) {
    throw new FailClosed(400, "invalid_body", `flow must be one of ${SYNC_FLOWS.join(", ")}`);
  }
  const marker = requiredString(body, "marker");
  if (!MARKER_PATTERN.test(marker)) {
    throw new FailClosed(
      400,
      "invalid_body",
      "marker must be a bounded token ([A-Za-z0-9._/-], at most 128 chars, no whitespace)",
    );
  }
  let labels: FlowSyncRequest["labels"] = null;
  const labelsRaw = body["labels"];
  if (labelsRaw !== undefined && labelsRaw !== null) {
    if (typeof labelsRaw !== "object" || Array.isArray(labelsRaw)) {
      throw new FailClosed(400, "invalid_body", "labels must be an object with add/remove lists");
    }
    const parseList = (field: string): string[] => {
      const value = (labelsRaw as Record<string, unknown>)[field];
      if (value === undefined || value === null) return [];
      if (!Array.isArray(value) || value.some((v) => typeof v !== "string" || v.trim() === "")) {
        throw new FailClosed(400, "invalid_body", `labels.${field} must be a list of non-empty strings`);
      }
      return value as string[];
    };
    labels = { add: parseList("add"), remove: parseList("remove") };
  }
  return {
    flow: flowRaw as SyncFlow,
    repo: requiredRepo(body, "repo"),
    number: requiredPositiveInt(body, "number"),
    kind: optionalKind(body),
    marker,
    body: requiredString(body, "body"),
    labels,
  };
}

/** The marker comment a flow's note carries. Dedupe is an exact
 * substring check over the item's existing comments: the same
 * flow + marker already posted is the retry-safety proof. */
export function flowMarkerComment(flow: SyncFlow, marker: string): string {
  return `<!-- fleet-sync:${flow}:${marker} -->`;
}

export interface FlowSyncResponse {
  flow: SyncFlow;
  repo: string;
  kind: "issue" | "pr";
  number: number;
  binding_id: string;
  comment: { outcome: "posted" | "already_posted"; url?: string };
  labels: { added: string[]; removed: string[] } | null;
  retires: string[];
}

export async function runFlowSync(
  deps: FlowSyncDeps,
  bindingId: string,
  request: FlowSyncRequest,
): Promise<FlowSyncResponse> {
  const markerComment = flowMarkerComment(request.flow, request.marker);
  const existing = await deps.comments.listIssueComments(request.repo, request.number);
  let comment: FlowSyncResponse["comment"];
  if (existing.some((c) => c.body.includes(markerComment))) {
    comment = { outcome: "already_posted" };
  } else {
    const posted = await deps.comments.postIssueComment(
      request.repo,
      request.number,
      `${request.body}\n\n${markerComment}`,
    );
    comment = { outcome: "posted", url: posted.url };
  }
  // Label edits are set operations (idempotent by construction),
  // so they run on every attempt: a retry whose predecessor
  // posted the note but failed the label flip still finishes it.
  let labels: FlowSyncResponse["labels"] = null;
  if (request.labels !== null) {
    await deps.comments.editIssueLabels(request.repo, request.number, request.labels.add, request.labels.remove);
    labels = { added: request.labels.add, removed: request.labels.remove };
  }
  return {
    flow: request.flow,
    repo: request.repo,
    kind: request.kind,
    number: request.number,
    binding_id: bindingId,
    comment,
    labels,
    retires: FLOW_SYNC_RETIRES,
  };
}
