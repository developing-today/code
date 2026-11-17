// Store backend selection for the four aggregator stores that became
// Supabase-backed in #6815 (actions, activity-ingestion, usage
// snapshots, aa-last-good).
//
// `PM_AGG_STORE_BACKEND` is the single switch:
//   - "file" (default): the historical laptop-local files. This is the
//     dev/test path and the rollback target.
//   - "supabase": the canonical muse-dev2 tables from
//     shared-schema/migrations/20261116_0001_aggregator_remote_stores.up.sql
//     (plus public.agg_actions from 20261105_0001).
//
// Exactly one backend is active. The stores never dual-write: in
// supabase mode the local files are not read or written by the store
// classes, and in file mode no database call is made. An unavailable
// Supabase client in supabase mode is an error, never a silent fallback
// to a local canonical copy.
export type StoreBackendKind = "file" | "supabase";

export function parseStoreBackend(env: Record<string, string | undefined> = process.env): StoreBackendKind {
  const raw = (env.PM_AGG_STORE_BACKEND ?? "").trim();
  if (raw === "" || raw === "file") return "file";
  if (raw === "supabase") return "supabase";
  throw new Error(
    `Invalid PM_AGG_STORE_BACKEND=${JSON.stringify(env.PM_AGG_STORE_BACKEND)}; expected "file" or "supabase"`,
  );
}

/** Recorded on every remote write so the single writer is auditable. */
export function storeWriterIdentity(env: Record<string, string | undefined> = process.env): string {
  const raw = (env.PM_AGG_STORE_WRITER ?? "").trim();
  return raw || "pm-aggregator";
}

/** Stable document keys in public.agg_store_documents. */
export const AA_LAST_GOOD_DOCUMENT = "aa-last-good";
