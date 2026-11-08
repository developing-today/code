// Supabase access for the aggregator — the exact path the fleet-feed
// collector uses for muse-dev2 (scripts/collect-fleet-feed.ts): SQL
// over the Management API query endpoint, authenticated with the
// Bitwarden secret named `supabase.access-token-muse-cli` (the sbp_
// management token), project ref from $SUPABASE_PROJECT_REF, else the
// Bitwarden `supabase.project-ref` secret, else the muse-dev2 ref.
// The token is only ever sent to api.supabase.com; it is never logged.
import { resolveBwsSecret } from "./secrets.ts";

/** Escape a string as a SQL string literal (standard-conforming strings:
 * doubling the single quote is sufficient). */
export function sqlStr(v: string): string {
  return `'${v.replaceAll("'", "''")}'`;
}

/** Escape a value as a jsonb literal. */
export function sqlJson(v: unknown): string {
  return `${sqlStr(JSON.stringify(v ?? null))}::jsonb`;
}

/** Validate + render an integer for SQL interpolation. */
export function sqlInt(v: number): string {
  if (!Number.isFinite(v) || Math.floor(v) !== v) throw new Error(`not an integer: ${v}`);
  return String(v);
}

export interface SupabaseClient {
  ref: string;
  query<T = Record<string, unknown>>(sql: string): Promise<T[]>;
}

async function supabaseQuery(token: string, ref: string, sql: string): Promise<unknown> {
  const res = await fetch(`https://api.supabase.com/v1/projects/${ref}/database/query`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({ query: sql }),
    signal: AbortSignal.timeout(60_000),
  });
  if (!res.ok) throw new Error(`Supabase query failed (HTTP ${res.status})`);
  return res.json();
}

let clientPromise: Promise<SupabaseClient | null> | null = null;

/**
 * The shared muse-dev2 client, resolved once. Null when no management
 * token is reachable (callers fail soft — the alerts surface reports
 * itself unavailable instead of taking the dashboard down).
 */
export function getSupabaseClient(): Promise<SupabaseClient | null> {
  if (!clientPromise) {
    clientPromise = (async () => {
      const token = await resolveBwsSecret("supabase.access-token-muse-cli");
      if (!token) return null;
      const ref =
        process.env.SUPABASE_PROJECT_REF ?? (await resolveBwsSecret("supabase.project-ref")) ?? "keihfchslgjetkmnmzra";
      return {
        ref,
        async query<T = Record<string, unknown>>(sql: string): Promise<T[]> {
          const out = await supabaseQuery(token, ref, sql);
          return (Array.isArray(out) ? out : []) as T[];
        },
      };
    })().catch(() => null);
  }
  return clientPromise;
}
