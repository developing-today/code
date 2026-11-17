// Path to Phase 12 panel data (#9922, split from #1239): the ordered
// step list lives in the BODY of issue #1235 (developing-today/code),
// and no other aggregator source carries issue bodies — so this module
// is the one-issue body feed for that panel, beside the Projects
// GraphQL reader in projects.ts (same injectable-fetch shape).
//
// Everything except fetchIssueBodySnapshot is pure: the GraphQL
// response parser, the step parser for #1235's checklist grammar, and
// the joins against the aggregator's GhItem states and — since the
// fleet-queue collector landed (#9920) — the queue tasks serving each
// step's issue, joined by the collector's dedupe-key/payload join.
import type { GhItem, RefKey } from "./model.ts";
import type { QueueSnapshot, QueueTask } from "./queue.ts";

export const PHASE12_REPO = "developing-today/code";
export const PHASE12_ISSUE_NUMBER = 1235;
export const PHASE12_ISSUE_URL = `https://github.com/${PHASE12_REPO}/issues/${PHASE12_ISSUE_NUMBER}`;
/** A fetched snapshot older than this renders with the stale warning. */
export const PHASE12_STALE_MS = 15 * 60_000;

/** One body fetch of #1235: the raw body plus provenance timestamps. */
export interface Phase12Snapshot {
  body: string;
  /** The issue's own updatedAt on GitHub, as reported by the fetch. */
  issueUpdatedAt: string;
  /** When this snapshot was fetched (ISO). The freshness stamp. */
  fetchedAt: string;
}

export const ISSUE_BODY_QUERY = `query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    issue(number: $number) { body updatedAt }
  }
}`;

interface GqlIssueBodyPage {
  errors?: unknown;
  data?: {
    repository?: {
      issue?: { body?: unknown; updatedAt?: unknown } | null;
    } | null;
  };
}

/** Parse one GraphQL issue-body response. Throws on GraphQL errors or
 * a missing/misshaped issue, so callers fail soft to "unavailable"
 * instead of rendering an invented step list. */
export function parseIssueBodyResponse(json: unknown): { body: string; issueUpdatedAt: string } {
  const page = json as GqlIssueBodyPage;
  if (!page || typeof page !== "object") throw new Error("github GraphQL issue-body response is not an object");
  if (page.errors) throw new Error("github GraphQL returned errors");
  const issue = page.data?.repository?.issue;
  if (!issue || typeof issue.body !== "string" || typeof issue.updatedAt !== "string") {
    throw new Error("github GraphQL issue-body response is missing the issue body");
  }
  return { body: issue.body, issueUpdatedAt: issue.updatedAt };
}

/** Fetch an issue's body via GraphQL with a caller-supplied token
 * (#1235 by default; #9926's readiness panel reads #534's body
 * through the same feed). Throws on HTTP/GraphQL errors (never logs
 * the token), mirroring fetchProjectPool in projects.ts. */
export async function fetchIssueBodySnapshot(
  token: string,
  fetchFn: typeof fetch = fetch,
  now: () => Date = () => new Date(),
  issueNumber: number = PHASE12_ISSUE_NUMBER,
): Promise<Phase12Snapshot> {
  const [owner, name] = PHASE12_REPO.split("/");
  const res = await fetchFn("https://api.github.com/graphql", {
    method: "POST",
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
      "User-Agent": "pm-aggregator-phase12",
    },
    body: JSON.stringify({
      query: ISSUE_BODY_QUERY,
      variables: { owner, name, number: issueNumber },
    }),
    signal: AbortSignal.timeout(20_000),
  });
  if (!res.ok) throw new Error(`github GraphQL HTTP ${res.status}`);
  const parsed = parseIssueBodyResponse(await res.json());
  return { body: parsed.body, issueUpdatedAt: parsed.issueUpdatedAt, fetchedAt: now().toISOString() };
}

/** One ordered step parsed from #1235's checklist. */
export interface Phase12Step {
  /** "Step 1" … "Step 14", or "Continuous". */
  label: string;
  /** The checkbox state in the body ([x] vs [ ]). */
  checked: boolean;
  /** The step's issue number in PHASE12_REPO. */
  issueNumber: number;
  /** The step title as written in the body, DONE annotation removed. */
  title: string;
  /** The trailing "DONE <date> (<evidence>)" annotation, when present. */
  doneNote: string | null;
}

// #1235's grammar (verified against its real body, 2026-11-14):
//   - [x] Step 1 — #229 Stabilize … — DONE 2026-10-26 (queue worker, PR #987, …)
//   - [ ] Continuous — #534 Phase 12 design workflow (designs land …)
const STEP_LINE = /^- \[( |x|X)\] (Step \d+|Continuous) — #(\d+) (.+)$/;
const DONE_SPLIT = /^(.*?) — (DONE .+)$/;

/** Parse the ordered steps from #1235's body. Non-step lines are
 * ignored; a body with no step lines parses to an empty list, which
 * the panel renders as unavailable rather than as "no steps". */
export function parsePhase12Steps(body: string): Phase12Step[] {
  const steps: Phase12Step[] = [];
  for (const line of body.split(/\r?\n/)) {
    const m = STEP_LINE.exec(line.trim());
    if (!m) continue;
    const issueNumber = Number(m[3]);
    if (!Number.isSafeInteger(issueNumber) || issueNumber <= 0) continue;
    const rest = m[4].trim();
    const done = DONE_SPLIT.exec(rest);
    steps.push({
      label: m[2],
      checked: m[1] !== " ",
      issueNumber,
      title: done ? done[1].trim() : rest,
      doneNote: done ? done[2].trim() : null,
    });
  }
  return steps;
}

/** Queue tasks grouped by the repo-qualified ref of the issue they
 * serve, newest first. The collector (#9920) already resolved each
 * task's repo + issue from its dedupe key (payload as corroboration);
 * tasks it could not join name no issue and cannot belong to a step,
 * so they are skipped here rather than guessed at. */
export function phase12QueueByRef(queue: QueueSnapshot | null): Map<RefKey, QueueTask[]> {
  const out = new Map<RefKey, QueueTask[]>();
  if (!queue) return out;
  for (const task of queue.tasks) {
    if (!task.repo || task.issue === null) continue;
    const ref: RefKey = `${task.repo}#${task.issue}`;
    const list = out.get(ref);
    if (list) list.push(task);
    else out.set(ref, [task]);
  }
  for (const list of out.values()) list.sort((a, b) => b.id - a.id);
  return out;
}

/** A step joined with the live GitHub state the aggregator resolved. */
export interface Phase12StepState extends Phase12Step {
  ref: RefKey;
  url: string;
  /** GhItem state for the step's issue; null = the source could not
   * resolve the ref, which the panel renders as "not resolved",
   * never as an invented state. */
  liveState: GhItem["state"] | null;
  /** The live item's title when resolved (the body's title is the
   * fallback and the historical record). */
  liveTitle: string | null;
  /** Queue tasks serving this step's issue (newest first), via the
   * collector's join. Empty when no task names the issue — the panel
   * renders that as "no queue task", never as an invented state. */
  queueTasks: QueueTask[];
}

/** Join parsed steps to GhItem states by repo-qualified ref, and to
 * queue tasks when a collector snapshot is supplied. */
export function joinPhase12Steps(
  steps: Phase12Step[],
  items: ReadonlyMap<RefKey, GhItem>,
  queue: QueueSnapshot | null = null,
): Phase12StepState[] {
  const queueByRef = phase12QueueByRef(queue);
  return steps.map((step) => {
    const ref: RefKey = `${PHASE12_REPO}#${step.issueNumber}`;
    const item = items.get(ref);
    return {
      ...step,
      ref,
      url: item?.url ?? `https://github.com/${PHASE12_REPO}/issues/${step.issueNumber}`,
      liveState: item ? item.state : null,
      liveTitle: item ? item.title : null,
      queueTasks: queueByRef.get(ref) ?? [],
    };
  });
}

/** Freshness for the panel: a snapshot is stale when its fetch is
 * older than PHASE12_STALE_MS (or unparseable — an unreadable stamp
 * is stale, never fresh). */
export function phase12IsStale(snapshot: Phase12Snapshot, now: Date = new Date()): boolean {
  const fetched = Date.parse(snapshot.fetchedAt);
  if (!Number.isFinite(fetched)) return true;
  return now.getTime() - fetched > PHASE12_STALE_MS;
}
