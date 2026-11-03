// Shared enqueue-failure handling for pm-dispatch (issue #4925).
//
// Both POST /v1/sessions and POST /v1/tasks create their GitHub
// tracking issue BEFORE enqueueing the queue task (the packet's
// canonical dedupe key names the issue number). When the enqueue — or
// the packet validation just before it — then fails, the old code
// returned the error with the issue left open and unannotated: a
// spec-less placeholder whose footer promised task/session ids that
// would never arrive. 59 open "do the thing" issues accumulated that
// way, each then consuming a coverage triage lane.
//
// This helper is the enqueue-side mirror of launch_failure.ts: when a
// dispatch fails before its task exists, an issue THIS dispatch
// created is commented with the failure and closed as not planned, so
// nothing dangles. Each step is best-effort and individually guarded —
// a comment failure must not suppress the close — and the outcome is
// returned so the HTTP response can say exactly what happened to the
// issue. Callers must only pass an issue the same request created:
// an issue attached via `issue_number` belongs to the caller and is
// never closed here.

/** The slice of the GitHub client enqueue-failure handling needs. */
export interface EnqueueFailureGitHub {
  comment(repo: string, issueNumber: number, body: string): Promise<void>;
  closeIssue(repo: string, issueNumber: number): Promise<void>;
}

export interface EnqueueFailureOutcome {
  commented: boolean;
  closed: boolean;
}

export async function handleEnqueueFailure(
  github: EnqueueFailureGitHub,
  repo: string,
  issueNumber: number,
  stage: string,
  e: unknown,
): Promise<EnqueueFailureOutcome> {
  const msg = e instanceof Error ? e.message : String(e);
  let commented = false;
  try {
    await github.comment(
      repo,
      issueNumber,
      `Dispatch FAILED at the ${stage} step, before any queue task existed for this issue.\n\nError: ${msg}\n\nClosing as not planned so this issue does not dangle as a spec-less placeholder (#4925). Re-dispatch with a fuller prompt if the work is still wanted.`,
    );
    commented = true;
  } catch {
    /* best-effort — the close below must still run */
  }
  let closed = false;
  try {
    await github.closeIssue(repo, issueNumber);
    closed = true;
  } catch {
    /* best-effort — the caller reports the outcome on the response */
  }
  return { commented, closed };
}
