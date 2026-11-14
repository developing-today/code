// Supabase access for the aggregator — the same Management API path the
// fleet-feed collector uses (scripts/collect-fleet-feed.ts): SQL
// over the Management API query endpoint, authenticated with the
// Bitwarden secret named `supabase.muse-dev2.access-token-muse-cli` (the sbp_
// management token), project ref from $SUPABASE_PROJECT_REF or the canonical
// Bitwarden `supabase.project-ref` pointer.
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

/** Resolve the current project ref without pinning a project in source. */
export async function resolveSupabaseProjectRef(
  resolveSecret: typeof resolveBwsSecret = resolveBwsSecret,
  env: Readonly<Record<string, string | undefined>> = process.env,
): Promise<string | null> {
  const override = env.SUPABASE_PROJECT_REF?.trim();
  if (override) return override;
  const ref = await resolveSecret("supabase.project-ref");
  return ref?.trim() || null;
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

/**
 * Create the shared Supabase client resolver. In-flight calls share one
 * attempt and successful clients stay cached; a null result is fail-soft
 * but not permanent, so a later call can recover.
 * Lazy-init contract (success-only memoization): README "Lazy-initialization contract" (#3879).
 */
export function createSupabaseClientGetter(
  resolveSecret: typeof resolveBwsSecret = resolveBwsSecret,
): () => Promise<SupabaseClient | null> {
  let clientPromise: Promise<SupabaseClient | null> | null = null;

  return function getSupabaseClient(): Promise<SupabaseClient | null> {
    if (!clientPromise) {
      const pending = (async () => {
        const token = await resolveSecret("supabase.muse-dev2.access-token-muse-cli");
        if (!token) return null;
        const ref = await resolveSupabaseProjectRef(resolveSecret);
        if (!ref) return null;
        return {
          ref,
          async query<T = Record<string, unknown>>(sql: string): Promise<T[]> {
            const out = await supabaseQuery(token, ref, sql);
            return (Array.isArray(out) ? out : []) as T[];
          },
        };
      })().catch(() => null);
      clientPromise = pending;
      void pending.then((client) => {
        if (client === null && clientPromise === pending) clientPromise = null;
      });
    }
    return clientPromise;
  };
}

/** The shared Supabase client, fail-soft while its management token or project ref is unavailable. */
export const getSupabaseClient = createSupabaseClientGetter();
