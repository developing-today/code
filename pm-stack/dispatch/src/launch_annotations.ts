// Post-claim launch annotations for POST /v1/sessions (#2660, design
// #2668 phase 2).
//
// The claim PATCH in launchSession is the launch's commit point: the
// session is already running and the task already claimed. Every step
// after it — the issue comment and the issue labels — is therefore
// individually guarded: a failure is recorded as a task note
// (last_error) and returned as a response warning, the launch is never
// rejected, and the remaining annotation steps still run. Before this
// module the comment and the label call shared one try/catch, so a
// failed comment silently skipped the labels and wait:true callers saw
// a clean success with no warning.

/** The slice of the queue client launch annotations need. */
export interface LaunchAnnotationQueue {
  patchTask(id: number, fields: Record<string, unknown>): Promise<unknown>;
  /** Optional read of the task row: when present, a pre-existing
   * last_error (e.g. a lane-ledger note PATCHed earlier in the same
   * launch) is preserved instead of overwritten. Absence (older
   * stubs) or failure degrades to the previous overwrite behavior —
   * the launch itself is never affected. */
  getTask?(id: number): Promise<Record<string, unknown> | null>;
}

/** The slice of the GitHub client launch annotations need. */
export interface LaunchAnnotationGitHub {
  comment(repo: string, issueNumber: number, body: string): Promise<void>;
  addLabels(repo: string, issueNumber: number, labels: string[]): Promise<void>;
}

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/**
 * Run the post-claim issue annotations, each guarded independently.
 * Returns one warning per failed annotation (empty when all succeed);
 * the wait:true path surfaces them in its success response. Each
 * failure is also noted on the task: the note accumulates, so when
 * both annotations fail the final last_error names both. A last_error
 * already on the task (e.g. the lane-ledger note from launchSession)
 * is preserved: the note is prepended, never overwritten.
 */
export async function annotateLaunch(
  github: LaunchAnnotationGitHub,
  queue: LaunchAnnotationQueue,
  repo: string,
  issueNumber: number,
  taskId: number,
  commentBody: string,
  labels: string[],
): Promise<string[]> {
  const warnings: string[] = [];
  const failures: string[] = [];

  // A last_error already on the task (the lane-ledger failure note is
  // PATCHed before annotateLaunch runs). Read lazily on the first
  // failure so the happy path issues no extra GET; a missing reader
  // or a failed read degrades to no prefix — best-effort throughout.
  let prefix: string | null = null;
  let prefixLoaded = false;
  async function loadPrefix(): Promise<void> {
    if (prefixLoaded) return;
    prefixLoaded = true;
    try {
      const row = await queue.getTask?.(taskId);
      const prior = row?.last_error;
      if (typeof prior === "string" && prior.trim()) prefix = prior.trim();
    } catch {
      /* best-effort: the annotation warning survives regardless */
    }
  }

  async function noteFailure(step: string, e: unknown): Promise<void> {
    const warning = `launch annotation failed (launch unaffected): ${step}: ${errorText(e)}`;
    warnings.push(warning);
    failures.push(warning);
    await loadPrefix();
    const note = prefix ? [prefix, ...failures].join("; ") : failures.join("; ");
    try {
      await queue.patchTask(taskId, { last_error: note });
    } catch {
      /* nothing more to do — the claim stands and the warning remains */
    }
  }

  try {
    await github.comment(repo, issueNumber, commentBody);
  } catch (e) {
    await noteFailure("issue comment", e);
  }

  try {
    await github.addLabels(repo, issueNumber, labels);
  } catch (e) {
    await noteFailure("issue labels", e);
  }

  return warnings;
}
