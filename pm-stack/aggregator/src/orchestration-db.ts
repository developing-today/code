// Orchestration reader flip (#10504, phase 1 of #3142): the fleet-feed
// collector's orchestration section is read from the latest
// public.orch_runs.document on Supabase muse-dev2 — the writer cutover
// (task 14838, PR #10381) has written every hourly pass there since
// 2026-11-15 — and orchestration.json, which the writer still
// regenerates every pass, becomes the export + rollback path only.
//
// Fallback contract (from #10504): on any DB read failure the reader
// serves the last good document — the file, which is never behind
// because the writer's step 3 keeps regenerating it from the stored
// document — and surfaces the failure; a partial or invalid DB
// document is never served. Rollback is ORCHESTRATION_SOURCE=file,
// which points the reader back at the file and loses nothing.
//
// This module is the decision logic only: the collector injects its
// own Supabase query route (Management API SQL, the same one the AA
// section uses) and its file read, so the flip is unit-testable
// without a database.

/** The latest run's complete document, newest first. generated_at is
 * the document's own generatedAt (writer contract, ORCHESTRATION.md);
 * run_id breaks ties deterministically. */
export const LATEST_ORCH_RUN_SQL =
  "SELECT run_id, document FROM public.orch_runs ORDER BY generated_at DESC, run_id DESC LIMIT 1";

/** Extract the document from a Management API result for
 * LATEST_ORCH_RUN_SQL. Returns null for anything that is not exactly
 * one usable row shape (no rows, a non-object document, a missing
 * run_id): the caller treats null as a DB read failure and falls
 * back — a partial document is never served. */
export function orchestrationDocumentFromRows(rows: unknown): { runId: string; document: unknown } | null {
  if (!Array.isArray(rows) || rows.length === 0) return null;
  const row: unknown = rows[0];
  if (typeof row !== "object" || row === null) return null;
  const { run_id: runId, document } = row as { run_id?: unknown; document?: unknown };
  if (typeof runId !== "string" || runId.length === 0) return null;
  if (typeof document !== "object" || document === null || Array.isArray(document)) return null;
  return { runId, document };
}

/** Which source the reader consults first. "file" is the rollback
 * position (#10504): the export the writer still regenerates every
 * pass, so pointing back at it loses nothing. Anything but the exact
 * string "file" selects the DB, the post-flip default. */
export function orchestrationSourceFromEnv(env: Readonly<Record<string, string | undefined>>): "db" | "file" {
  return env.ORCHESTRATION_SOURCE?.trim().toLowerCase() === "file" ? "file" : "db";
}

export interface OrchestrationResolution {
  /** The document to serve, or null when neither source produced one. */
  document: unknown | null;
  /** Where the served document came from; null when none was served. */
  source: "db" | "file" | null;
  /** Why the DB read failed, when it was attempted and failed. Set
   * alongside a file-sourced document so the caller can surface the
   * failure while serving the last good document. */
  dbError?: string;
  /** Why the file read failed, when it was attempted and failed. */
  fileError?: string;
  /** True when the file read failed because the file does not exist
   * (the pre-#899 normal state), as opposed to an unreadable file. */
  fileMissing?: boolean;
}

const errorText = (err: unknown): string => (err instanceof Error ? err.message : "read failed");

const isMissing = (err: unknown): boolean =>
  typeof err === "object" && err !== null && "code" in err && (err as { code?: unknown }).code === "ENOENT";

/**
 * Resolve the orchestration document per the flip contract.
 *
 * - DB source (default): read the latest orch_runs.document and
 *   validate it with `validate` (the shared structural check —
 *   orchestrationSectionProblem). A read failure *or* an invalid
 *   document is a DB failure: fall back to the file and carry the
 *   reason in `dbError` so it is surfaced, never hidden.
 * - File source (rollback): the DB is not consulted at all.
 * - The file read's own failure is reported, not hidden: `fileError`,
 *   plus `fileMissing` for the not-yet-published case, so the caller
 *   keeps today's skipped-vs-error distinction.
 */
export async function resolveOrchestrationDocument(deps: {
  source: "db" | "file";
  readDb: () => Promise<unknown>;
  readFile: () => Promise<unknown>;
  validate: (document: unknown) => string | null;
}): Promise<OrchestrationResolution> {
  let dbError: string | undefined;
  if (deps.source === "db") {
    try {
      const document = await deps.readDb();
      const problem = deps.validate(document);
      if (problem === null) return { document, source: "db" };
      dbError = `latest orch_runs.document failed its structural check (${problem})`;
    } catch (err) {
      dbError = errorText(err);
    }
  }
  try {
    const document = await deps.readFile();
    return { document, source: "file", ...(dbError ? { dbError } : {}) };
  } catch (err) {
    return {
      document: null,
      source: null,
      ...(dbError ? { dbError } : {}),
      fileError: errorText(err),
      fileMissing: isMissing(err),
    };
  }
}
