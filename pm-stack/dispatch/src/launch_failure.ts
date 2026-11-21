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
//
// #4917: the tracking issue itself is also closed as not planned on
// every failure path. A failed launch's issue is a record of a dispatch
// that never became real; leaving it open produced 66 orphan
// 'do the thing' issues (#3499-#4899). Failures BEFORE the enqueue
// (packet validation, enqueue itself) have no task to cancel — those
// callers use compensateOrphanedIssue below, which performs the same
// note + close for the issue they created. Only issues dispatch
// created in the same request are ever closed here; attach targets
// (caller-supplied issue numbers) are never compensated.
import { ClaimLostError } from "./queue.ts";

/** The slice of the queue client launch-failure handling needs. */
export interface LaunchFailureQueue {
  cancelTask(id: number, reason: string): Promise<void>;
}

/** The slice of the GitHub client launch-failure handling needs. */
export interface LaunchFailureGitHub {
  comment(repo: string, issueNumber: number, body: string): Promise<void>;
  closeIssue(repo: string, issueNumber: number, stateReason?: "not_planned" | "completed"): Promise<void>;
}

/**
 * Compensate a tracking issue whose dispatch failed BEFORE a queue
 * task existed (#4917): note the failure on the issue and close it as
 * not planned, so it does not linger open as an orphan. Both steps are
 * best-effort and individually guarded; the original error message is
 * returned for the caller's error response. A null issue number (no
 * issue was created) is a no-op.
 */
export async function compensateOrphanedIssue(
  github: LaunchFailureGitHub,
  repo: string,
  issueNumber: number | null,
  stage: string,
  e: unknown,
): Promise<string> {
  const msg = e instanceof Error ? e.message : String(e);
  if (issueNumber == null) return msg;
  try {
    await github.comment(
      repo,
      issueNumber,
      `Dispatch launch FAILED at ${stage} before a queue task was created, so this dispatch never became real. Closing this tracking issue as not planned so it does not linger as an orphan (#4917).\n\nError: ${msg}`,
    );
  } catch {
    /* issue note is best-effort on the failure path */
  }
  try {
    await github.closeIssue(repo, issueNumber, "not_planned");
  } catch {
    /* close is best-effort; the note above still records the failure */
  }
  return msg;
}

/** Mutable context shared by launchSession and its failure handler. */
export interface LaunchFailureContext {
  sessionId: string | null;
  /** The dispatch worktree created for this launch, if any (#12217).
   * Set the moment the tree exists — before the session — so a failure
   * at session creation can still clean the tree up. */
  worktree: string | null;
}

/** Run once the launch's session is CONFIRMED gone (dispatch itself
 * deleted it), or — when no session was ever recorded — after the
 * failure was established. Returns the cleanup outcome for the note,
 * or null when there is nothing to retire. Never throws: a worktree
 * outcome must not mask the launch failure being reported. */
export type RetireWorktree = (sessionId?: string | null) => Promise<{ worktree: string; removed: boolean; reason: string } | null>;
export type ReportRetirement = (outcome: { worktree: string; removed: boolean; reason: string }) => void;

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
  retireWorktree?: RetireWorktree,
  reportRetirement: ReportRetirement = (outcome) => {
    if (!outcome.removed) {
      console.warn(`[pm-dispatch] task worktree ${outcome.worktree} retained: ${outcome.reason}`);
    }
  },
): Promise<string> {
  const msg = e instanceof Error ? e.message : String(e);
  // #12217: best-effort worktree retirement, reported, never thrown —
  // and never attempted while the session may still be live.
  const retireSafely = async (retiredSessionId: string | null = sessionId): Promise<{ worktree: string; removed: boolean; reason: string } | null> => {
    if (!retireWorktree) return null;
    try {
      const outcome = await retireWorktree(retiredSessionId);
      if (outcome && !outcome.removed) safelyReportRetirement(outcome);
      return outcome;
    } catch (re) {
      const reason = re instanceof Error ? re.message : String(re);
      const outcome = { worktree: "unknown", removed: false, reason };
      safelyReportRetirement(outcome);
      return outcome;
    }
  };
  const safelyReportRetirement = (outcome: { worktree: string; removed: boolean; reason: string }) => {
    try {
      reportRetirement(outcome);
    } catch {
      /* reporting must not mask the original launch failure */
    }
  };
  if (e instanceof ClaimLostError) {
    // Lost the claim race (#10737): the task and its issue now belong
    // to the worker that claimed it, so neither is cancelled, noted or
    // closed. Only this launch's own session is cleaned up.
    if (sessionId != null && deleteSession) {
      try {
        await deleteSession(sessionId);
        await retireSafely(sessionId);
      } catch {
        // Deletion may have failed while the session is still live. Run
        // the fail-closed evidence check so the retained worktree is visible.
        await retireSafely(sessionId);
      }
    }
    return msg;
  }
  const sessionContext = sessionId == null ? "" : `; OpenCode session ${sessionId} may still be live`;
  let cancelError: string | null = null;
  try {
    await queue.cancelTask(taskId, `dispatch launch failed: ${msg}${sessionContext}`);
  } catch (ce) {
    /* cancellation failed too — the note below must say so, not claim success */
    cancelError = ce instanceof Error ? ce.message : String(ce);
  }
  let cleanupError: string | null = null;
  let worktreeNote = "";
  if (sessionId == null) {
    // createSession can time out or lose its response after the server
    // accepted the request. The session may still come up later, so the
    // tree cannot be retired on an empty session-list snapshot.
    const outcome = retireWorktree
      ? await retireSafely(null) ?? {
          worktree: "unknown",
          removed: false,
          reason: "session creation outcome is unknown; a session may still be live",
        }
      : null;
    if (outcome) {
      worktreeNote = outcome.removed
        ? `\n\nTask worktree ${outcome.worktree} was removed after the failed launch.`
        : `\n\nTask worktree ${outcome.worktree} was retained: ${outcome.reason}.`;
    }
  } else if (!deleteSession) {
    cleanupError = "session deletion callback unavailable";
  } else {
    try {
      await deleteSession(sessionId);
      // The session is confirmed gone: only now may the worktree be
      // considered for retirement (#12217).
      const outcome = await retireSafely();
      if (outcome) {
        worktreeNote = outcome.removed
          ? `\n\nTask worktree ${outcome.worktree} was removed after the failed launch.`
          : `\n\nTask worktree ${outcome.worktree} was retained: ${outcome.reason}.`;
      }
    } catch (de) {
      cleanupError = de instanceof Error ? de.message : String(de);
      const outcome = await retireSafely(sessionId);
      if (outcome) {
        worktreeNote = outcome.removed
          ? `\n\nTask worktree ${outcome.worktree} was removed after the failed launch.`
          : `\n\nTask worktree ${outcome.worktree} was retained: ${outcome.reason}.`;
      }
    }
  }
  if (issueNumber != null) {
    const sessionNote =
      sessionId == null
        ? ""
        : cleanupError == null
          ? `\n\nOpenCode session ${sessionId} was deleted; session cleanup succeeded.`
          : `\n\nOpenCode session ${sessionId} may still be live; session cleanup failed: ${cleanupError}`;
    const note =
      cancelError == null
        ? `Dispatch launch FAILED and queue task #${taskId} was cancelled.${sessionNote}${worktreeNote}\n\nError: ${msg}`
        : `Dispatch launch FAILED and cancelling queue task #${taskId} FAILED — the task may still be queued or claimed and needs cleanup.${sessionNote}${worktreeNote}\n\nError: ${msg}\nCancellation error: ${cancelError}`;
    try {
      await github.comment(repo, issueNumber, note);
    } catch {
      /* issue note is best-effort on the failure path */
    }
    // #4917: a failed launch's tracking issue must not stay open —
    // close it as not planned after noting the failure.
    try {
      await github.closeIssue(repo, issueNumber, "not_planned");
    } catch {
      /* close is best-effort; the note above still records the failure */
    }
  }
  return msg;
}
