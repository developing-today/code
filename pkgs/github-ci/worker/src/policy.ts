/**
 * Pure ingest policy, ported 1:1 from the github-ci Edge Function
 * (ACTIVE v8) so the two receivers are interchangeable during the
 * #5693 dual-run diff period.
 */
export const FLEET_REPOS = new Set([
  "developing-today/code",
  "developing-today/opencode",
  "developing-today/openchamber",
  "dezren39/nix",
]);

export interface RunRow {
  received_at: string;
  source: string;
  event_type: string;
  action: string;
  repo: string;
  workflow_name: string | null;
  workflow_id: number | null;
  run_id: number | null;
  run_attempt: number;
  branch: string | null;
  sha: string | null;
  status: string | null;
  conclusion: string | null;
  actor: string | null;
  html_url: string | null;
  ci_started_at: string | null;
  ci_updated_at: string | null;
  payload: string;
  dedupe_key: string;
}

/** v4 ingest policy: store only completed runs, never skipped. */
export function skipReason(action: string, conclusion: string | null): string | null {
  if (action !== "completed") return "not_completed";
  if (conclusion === "skipped") return "skipped_conclusion";
  return null;
}

export function dedupeKey(repo: string, runId: number | null, attempt: number, action: string): string {
  return `github:${repo}:${runId}:${attempt}:${action}`;
}

export function buildRunRow(
  payload: Record<string, unknown>,
  repoName: string,
  eventType: string,
  receivedAt: string,
): { row: RunRow; runId: number | null; action: string; conclusion: string | null } {
  const run = (payload.workflow_run ?? {}) as Record<string, unknown>;
  const runId = Number(run.id ?? 0) || null;
  const attempt = Number(run.run_attempt ?? 1) || 1;
  const action = String(payload.action ?? "");
  const conclusion = (run.conclusion as string | null) ?? null;
  const row: RunRow = {
    received_at: receivedAt,
    source: "github",
    event_type: eventType,
    action,
    repo: repoName,
    workflow_name: (run.name as string) ?? null,
    workflow_id: Number(run.workflow_id ?? 0) || null,
    run_id: runId,
    run_attempt: attempt,
    branch: (run.head_branch as string) ?? null,
    sha: (run.head_sha as string) ?? null,
    status: (run.status as string) ?? null,
    conclusion,
    actor: ((run.actor as Record<string, unknown> | undefined)?.login as string) ?? null,
    html_url: (run.html_url as string) ?? null,
    ci_started_at: (run.run_started_at as string) ?? (run.created_at as string) ?? null,
    ci_updated_at: (run.updated_at as string) ?? null,
    // v4 policy: raw payload kept only for failures (forensics);
    // every other stored row carries an empty object.
    payload: conclusion === "failure" ? JSON.stringify(payload) : "{}",
    dedupe_key: dedupeKey(repoName, runId, attempt, action),
  };
  return { row, runId, action, conclusion };
}
