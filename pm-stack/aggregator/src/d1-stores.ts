// Cloudflare D1's HTTP API transport for laptop-side PM store writers.
// D1 has no libSQL/Turso protocol endpoint: local processes use the
// Cloudflare API, while Workers should use their D1 binding directly.
import { resolveBwsSecret } from "./secrets.ts";
import { decodeStoreRow, storeStatement } from "./turso-stores.ts";
import type { SqlClient } from "./sql-client.ts";

export const D1_API_TOKEN_NAME = "cloudflare.api-token-account-wide";

type D1QueryResponse = {
  success?: boolean;
  result?: Array<{
    success?: boolean;
    results?: Record<string, unknown>[];
  }>;
};

/** Build a client for one D1 database using Cloudflare's documented query API. */
export function createD1StoreClient(
  accountId: string,
  databaseId: string,
  token: string,
  fetchFn: typeof fetch = fetch,
): SqlClient {
  if (!/^[a-f\d]{32}$/i.test(accountId)) throw new Error("invalid Cloudflare account ID");
  if (!/^[a-f\d-]{36}$/i.test(databaseId)) throw new Error("invalid D1 database ID");
  if (!token.trim()) throw new Error("missing Cloudflare API token");

  const endpoint = `https://api.cloudflare.com/client/v4/accounts/${accountId}/d1/database/${databaseId}/query`;
  return {
    ref: `d1:${databaseId}`,
    async query<T = Record<string, unknown>>(input: string): Promise<T[]> {
      const statement = storeStatement(input);
      const sql = statement.sql.replace(/\?(\d+)/g, "?");
      const response = await fetchFn(endpoint, {
        method: "POST",
        headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
        body: JSON.stringify({ sql, params: statement.args.map((arg) => arg.value) }),
        signal: AbortSignal.timeout(60_000),
      });
      if (!response.ok) throw new Error(`D1 store query failed (HTTP ${response.status})`);

      let body: D1QueryResponse;
      try {
        body = (await response.json()) as D1QueryResponse;
      } catch {
        throw new Error("D1 store query returned malformed JSON");
      }
      const result = body.result?.[0];
      if (body.success !== true || result?.success !== true || !Array.isArray(result.results)) {
        // Cloudflare error text can contain request details; keep it out of
        // logs because this transport is used by background laptop jobs.
        throw new Error("D1 store query failed or returned a malformed result");
      }
      return result.results.map((row) => decodeStoreRow(row) as T);
    },
  };
}

/** Resolve the configured D1 database and its token by secret name. */
export function createD1StoreGetter(
  resolveSecret = resolveBwsSecret,
  env: NodeJS.ProcessEnv = process.env,
): () => Promise<SqlClient | null> {
  let cached: Promise<SqlClient | null> | null = null;
  return () => {
    if (!cached) {
      const pending = (async () => {
        const accountId = env.CLOUDFLARE_ACCOUNT_ID;
        const databaseId = env.PM_AGG_D1_DATABASE_ID;
        if (!accountId || !databaseId) return null;
        const token = await resolveSecret(D1_API_TOKEN_NAME);
        return token ? createD1StoreClient(accountId, databaseId, token) : null;
      })();
      cached = pending;
      void pending.then(
        (client) => {
          if (!client && cached === pending) cached = null;
        },
        () => {
          if (cached === pending) cached = null;
        },
      );
    }
    return cached;
  };
}

export const getD1StoreClient = createD1StoreGetter();
