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
// Failure handling records cancellation first, then attempts session
// cleanup, then notes the issue. Each is best-effort: failure of one does
// not suppress the next. The note reports the cancellation and cleanup
// outcomes, and the original launch error is returned for the wait:true
// path's 502.

/** The slice of the queue client launch-failure handling needs. */
export interface LaunchFailureQueue {
  cancelTask(id: number, reason: string): Promise<void>;
}

/** The slice of the GitHub client launch-failure handling needs. */
export interface LaunchFailureGitHub {
  comment(repo: string, issueNumber: number, body: string): Promise<void>;
}

/** Mutable context shared by launchSession and its failure handler. */
export interface LaunchFailureContext {
  sessionId: string | null;
}

/**
 * Run the part of a launch that follows successful session creation.
 * Retain the new session ID so the failure handler can record it before
 * attempting cleanup.
 */
export async function withCreatedSession<T>(
  sessionId: string,
  context: LaunchFailureContext,
  launch: () => Promise<T>,
): Promise<T> {
  context.sessionId = sessionId;
  return launch();
}

export async function handleLaunchFailure(
  queue: LaunchFailureQueue,
  github: LaunchFailureGitHub,
  repo: string,
  taskId: number,
  issueNumber: number | null,
  e: unknown,
  sessionId: string | null = null,
  deleteSession?: (sessionId: string) => Promise<void>,
): Promise<string> {
  const msg = e instanceof Error ? e.message : String(e);
  const sessionContext = sessionId == null
    ? ""
    : `; OpenCode session ${sessionId} may still be live`;
  let cancelError: string | null = null;
  try {
    await queue.cancelTask(taskId, `dispatch launch failed: ${msg}${sessionContext}`);
  } catch (ce) {
    /* cancellation failed too — the note below must say so, not claim success */
    cancelError = ce instanceof Error ? ce.message : String(ce);
  }
  let cleanupError: string | null = null;
  if (sessionId != null) {
    if (!deleteSession) {
      cleanupError = "session deletion callback unavailable";
    } else {
      try {
        await deleteSession(sessionId);
      } catch (de) {
        cleanupError = de instanceof Error ? de.message : String(de);
      }
    }
  }
  if (issueNumber != null) {
    const sessionNote = sessionId == null
      ? ""
      : cleanupError == null
        ? `\n\nOpenCode session ${sessionId} may still be live; session cleanup succeeded.`
        : `\n\nOpenCode session ${sessionId} may still be live; session cleanup failed: ${cleanupError}`;
    const note =
      cancelError == null
        ? `Dispatch launch FAILED and queue task #${taskId} was cancelled.${sessionNote}\n\nError: ${msg}`
        : `Dispatch launch FAILED and cancelling queue task #${taskId} FAILED — the task may still be queued or claimed and needs cleanup.${sessionNote}\n\nError: ${msg}\nCancellation error: ${cancelError}`;
    try {
      await github.comment(repo, issueNumber, note);
    } catch {
      /* issue note is best-effort on the failure path */
    }
  }
  return msg;
}
