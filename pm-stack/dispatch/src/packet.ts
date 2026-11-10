// repo-task/v1 packet construction for the Supabase queue, mirroring the
// shape ~/tools/repo-queue.py validates: an envelope (title/body/priority/
// dedupe_key) around a repo-task/v1 payload. Dispatch adds `origin`,
// `caller`, `model_hint` and `directory` as extra payload fields (the
// schema is additionalProperties: true).
import type { CallerIdentity } from "./auth.ts";
import type { ModelRef, TaskKind } from "./validate.ts";

export function randomDispatchSuffix(): string {
  const bytes = new Uint8Array(4);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

/** Unique dispatch dedupe key for work with no GitHub issue to anchor to. */
export function dispatchDedupeKey(repo: string): string {
  return `repo-health:v1:triage:${repo}:dispatch:${randomDispatchSuffix()}`;
}

/** Canonical dedupe key families (repo-queue.py DEDUPE_PATTERNS_DOC). */
export function canonicalDedupeKey(kind: TaskKind, repo: string, issueNumber: number): string | null {
  switch (kind) {
    case "small_fix":
      return `repo-health:v1:fix:${repo}:issue:${issueNumber}`;
    case "triage":
    case "ci_investigation":
      return `repo-health:v1:triage:${repo}:issue:${issueNumber}`;
    case "pr_shepherd":
      // pr_shepherd keys anchor to PRs, not issues; without a PR in hand
      // there is no canonical key — caller falls back to a dispatch key.
      return null;
  }
}

export interface PacketInput {
  repo: string;
  kind: TaskKind;
  /** Queue-task title (envelope). */
  title: string;
  /** The work text: the session prompt / task body (envelope body too). */
  prompt: string;
  priority: number;
  caller: CallerIdentity;
  model: ModelRef | null;
  directory: string;
  issue: { number: number; url: string } | null;
}

export interface Packet {
  title: string;
  body: string;
  priority: number;
  dedupe_key: string;
  payload: Record<string, unknown>;
}

export function buildPacket(input: PacketInput, dedupeKey: string): Packet {
  const now = new Date().toISOString();
  const isTriage = input.kind === "triage";
  const payload: Record<string, unknown> = {
    schema: "repo-task/v1",
    kind: input.kind,
    repo: input.repo,
    problem:
      input.kind === "small_fix" || input.kind === "triage"
        ? `Dispatch-requested work for ${input.repo}: ${input.title}`
        : `Dispatch-requested ${input.kind} for ${input.repo}: ${input.title}`,
    evidence: [
      `Requested via pm-dispatch by ${describeCaller(input.caller)}`,
      input.issue
        ? `Tracking issue: ${input.issue.url}`
        : "No GitHub issue was created for this dispatch (issue=false or app not installed); this task is the tracking record.",
      `Directory: ${input.directory}`,
    ],
    acceptance:
      input.kind === "small_fix"
        ? [
            "Session launched and work completed per prompt; close the issue from the session or a follow-up",
          ]
        : ["Work described in the task body is triaged/completed per its prompt; findings recorded on the task or its issue"],
    scope: {
      include: [`${input.repo} — the work described in the task body`],
      exclude: ["Unrelated refactors or changes outside the described work"],
    },
    tests: ["As described in the task body; otherwise the repo's standard checks for the touched area"],
    risk: { level: "low", flags: [] },
    priority_reason: `Caller-supplied dispatch priority (resolved to ${input.priority} on the queue design scale).`,
    timebox_minutes: isTriage ? 20 : 45,
    max_attempts: isTriage || input.kind === "ci_investigation" ? 3 : 2,
    context_refs: input.issue ? [input.issue.url] : [],
    dedupe: {
      checked_at: now,
      basis: input.issue
        ? `Dispatch created tracking issue #${input.issue.number} for this work; canonical dedupe key anchors to it.`
        : "Dispatch without an issue: unique dispatch dedupe key (no canonical issue anchor exists).",
    },
    origin: "dispatch",
    caller: input.caller,
    model_hint: input.model,
    directory: input.directory,
    prompt: input.prompt,
  };
  if (input.issue) {
    payload.issue = {
      number: input.issue.number,
      url: input.issue.url,
      state_checked_at: now,
    };
  }
  return {
    title: input.title,
    body: input.prompt,
    priority: input.priority,
    dedupe_key: dedupeKey,
    payload,
  };
}

export function describeCaller(caller: CallerIdentity): string {
  if (caller.type === "bearer") return "bearer-token caller";
  return `GitHub Actions OIDC (repo ${caller.repository}${caller.actor ? `, actor ${caller.actor}` : ""}${caller.workflow_ref ? `, workflow ${caller.workflow_ref}` : ""})`;
}

/**
 * Self-check mirroring repo-queue.py's built-in payload validation, so a
 * malformed packet fails here with a clear message instead of at the RPC.
 * Returns a list of errors (empty = valid). Dispatch-specific extras
 * (origin/caller/…) are allowed, as the schema permits additional
 * properties; the identity rule accepts the dispatch origin in place of
 * an issue/prs/set/source anchor.
 */
export function validatePayload(payload: Record<string, unknown>): string[] {
  const errs: string[] = [];
  const req = (f: string) => {
    const v = payload[f];
    if (v === undefined || v === null || v === "" || (Array.isArray(v) && v.length === 0)) {
      errs.push(`payload.${f}: required field missing or empty`);
    }
  };
  if (payload.schema !== "repo-task/v1") errs.push("payload.schema: must be 'repo-task/v1'");
  if (!["pr_shepherd", "triage", "small_fix", "ci_investigation"].includes(String(payload.kind))) {
    errs.push("payload.kind: not a valid task kind");
  }
  for (const f of ["problem", "priority_reason"]) req(f);
  for (const f of ["evidence", "acceptance", "tests", "context_refs"]) {
    if (!Array.isArray(payload[f])) errs.push(`payload.${f}: must be a list`);
  }
  for (const f of ["evidence", "acceptance", "tests"]) {
    if (Array.isArray(payload[f]) && (payload[f] as unknown[]).length === 0) {
      errs.push(`payload.${f}: must not be empty`);
    }
  }
  const scope = payload.scope as Record<string, unknown> | undefined;
  if (!scope || !Array.isArray(scope.include) || !Array.isArray(scope.exclude) || scope.exclude.length === 0) {
    errs.push("payload.scope: must have include list and non-empty exclude list");
  }
  const risk = payload.risk as Record<string, unknown> | undefined;
  if (!risk || !["low", "medium", "high"].includes(String(risk.level)) || !Array.isArray(risk.flags)) {
    errs.push("payload.risk: must have level low|medium|high and flags list");
  }
  for (const f of ["timebox_minutes", "max_attempts"]) {
    if (!Number.isInteger(payload[f]) || (payload[f] as number) < 1) {
      errs.push(`payload.${f}: must be a positive integer`);
    }
  }
  const ded = payload.dedupe as Record<string, unknown> | undefined;
  if (!ded || !ded.checked_at || !ded.basis) errs.push("payload.dedupe: must have checked_at and basis");
  const hasIssue =
    typeof payload.issue === "object" && payload.issue !== null &&
    Number.isInteger((payload.issue as Record<string, unknown>).number);
  const hasDispatch = payload.origin === "dispatch";
  if (!hasIssue && !hasDispatch && !Array.isArray(payload.prs) && typeof payload.set !== "object") {
    errs.push("payload identity: need issue.number, prs, set, or dispatch origin");
  }
  return errs;
}
