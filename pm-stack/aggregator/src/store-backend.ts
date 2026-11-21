// Store backend selection for the four aggregator stores that moved off
// laptop-local files in #6815 (actions, activity-ingestion, usage
// snapshots, aa-last-good).
//
// `PM_AGG_STORE_BACKEND` is the single switch:
//   - "file" (default): the historical laptop-local files. This is the
//     dev/test path and the rollback target.
//   - "turso": the canonical Turso tables from
//     shared-schema/migrations/20261116_0001_aggregator_remote_stores.up.sql
//     (plus agg_actions from 20261105_0001), via src/turso-stores.ts.
//
// Exactly one backend is active. The stores never dual-write: in
// turso mode the local files are not read or written by the store
// classes, and in file mode no database call is made. An unavailable
// Turso client in turso mode is an error, never a silent fallback to a
// local canonical copy. Any other value (including the retired
// "supabase") fails startup loudly.
export type StoreBackendKind = "file" | "turso";

export function parseStoreBackend(env: Record<string, string | undefined> = process.env): StoreBackendKind {
  const raw = (env.PM_AGG_STORE_BACKEND ?? "").trim();
  if (raw === "" || raw === "file") return "file";
  if (raw === "turso") return "turso";
  throw new Error(
    `Invalid PM_AGG_STORE_BACKEND=${JSON.stringify(env.PM_AGG_STORE_BACKEND)}; expected "file" or "turso"`,
  );
}

/** Recorded on every remote write so the single writer is auditable. */
export function storeWriterIdentity(env: Record<string, string | undefined> = process.env): string {
  const raw = (env.PM_AGG_STORE_WRITER ?? "").trim();
  return raw || "pm-aggregator";
}

/** Stable document keys in public.agg_store_documents. */
export const AA_LAST_GOOD_DOCUMENT = "aa-last-good";
