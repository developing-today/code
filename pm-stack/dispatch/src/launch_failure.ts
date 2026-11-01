// Shared launch-failure handling for POST /v1/sessions (#2901).
//
// A launch that fails after the enqueue must never leave a queued or
// claimed task behind for a session that did not launch: the task is
// cancelled and the failure is noted on its tracking issue. The
// wait:true path has always done this; the wait:false path only
// PATCHed last_error onto the task, leaving it queued forever — so
// both paths now call this one helper, and the async path cannot
// drift from the synchronous one again.
//
// Both steps are best-effort: a cancellation failure does not suppress
// the issue note, and neither failure suppresses the other. The
// message is returned so the wait:true path can put it in its 502.

/** The slice of the queue client launch-failure handling needs. */
export interface LaunchFailureQueue {
  cancelTask(id: number, reason: string): Promise<void>;
}

/** The slice of the GitHub client launch-failure handling needs. */
export interface LaunchFailureGitHub {
  comment(repo: string, issueNumber: number, body: string): Promise<void>;
}

export async function handleLaunchFailure(
  queue: LaunchFailureQueue,
  github: LaunchFailureGitHub,
  repo: string,
  taskId: number,
  issueNumber: number | null,
  e: unknown,
): Promise<string> {
  const msg = e instanceof Error ? e.message : String(e);
  try {
    await queue.cancelTask(taskId, `dispatch launch failed: ${msg}`);
  } catch {
    /* cancellation failed too — note the launch error regardless */
  }
  if (issueNumber != null) {
    try {
      await github.comment(
        repo,
        issueNumber,
        `Dispatch launch FAILED and queue task #${taskId} was cancelled.\n\nError: ${msg}`,
      );
    } catch {
      /* issue note is best-effort on the failure path */
    }
  }
  return msg;
}
