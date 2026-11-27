// Turso transport for the PM stores migrated from muse-dev2. This is a
// deliberately bounded dialect bridge for remote-stores.ts and alerts.ts,
// not an arbitrary PostgreSQL query translator. Literals become typed args
// BEFORE rewriting syntax, so user text/JSON can never be rewritten as SQL.
import { resolveBwsSecret } from "./secrets.ts";
import type { SqlClient } from "./sql-client.ts";
import type { TursoWriteContext, TursoWriteMetrics } from "./turso-write-metrics.ts";
import { DECISION_EVENTS_TURSO_URL, DECISION_EVENTS_TURSO_TOKEN_NAME } from "./decision-events-sync.ts";

export type StoreArg = { type: "text"; value: string };
export function storeStatement(input: string): { sql: string; args: StoreArg[] } {
  const args: StoreArg[] = [];
  let sql = "";
  for (let i = 0; i < input.length; ) {
    if (input[i] !== "'") {
      sql += input[i++];
      continue;
    }
    let value = "";
    let closed = false;
    i++;
    while (i < input.length) {
      if (input[i] !== "'") {
        value += input[i++];
        continue;
      }
      if (input[i + 1] === "'") {
        value += "'";
        i += 2;
        continue;
      }
      i++;
      closed = true;
      break;
    }
    if (!closed) throw new Error("unterminated store SQL literal");
    args.push({ type: "text", value });
    sql += `?${args.length}`;
  }
  sql = sql
    .replace(/\bpublic\.(agg_[a-z_]+|shared_[a-z_]+|orch_runs)\b/g, "$1")
    .replace(/\balerts\.(incidents|incident_events|push_subscriptions|notification_sends)\b/g, "alerts_$1")
    .replace(/\bpush_subscriptions\.classes\b/g, "alerts_push_subscriptions.classes")
    // A text[] of literals becomes one JSON text argument rather than a
    // json_array() call: D1 caps a function at 127 arguments, and seen-id
    // lists run to thousands (#15671 follow-up).
    .replace(/array\[([^\]]*)\]::text\[\]/gi, (m, inner: string) => {
      const items = inner.split(",").map((s) => s.trim());
      if (!items.every((s) => /^\?\d+$/.test(s))) return `json_array(${inner})`;
      args.push({ type: "text", value: JSON.stringify(items.map((s) => args[Number(s.slice(1)) - 1].value)) });
      return `json(?${args.length})`;
    })
    .replace(/(\?\d+)::text\[\]/g, (_m, p) => {
      const n = Number(p.slice(1)) - 1;
      if (args[n].value !== "{}") throw new Error("unsupported PostgreSQL array literal");
      args[n].value = "[]";
      return p;
    })
    .replace(/::(?:jsonb|timestamptz)\b/g, "")
    .replace(
      /alerts_push_subscriptions\.classes\s*\|\|\s*(\?\d+)/g,
      "json_patch(alerts_push_subscriptions.classes, $1)",
    )
    .replace(
      /now\(\)\s*-\s*make_interval\(mins\s*=>\s*(\d+)\)/g,
      "strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-$1 minutes')",
    )
    .replace(/\bnow\(\)/g, "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')");
  // SQLite partial-index predicates cannot contain parameters. This is a
  // fixed schema predicate, not arbitrary literal interpolation.
  sql = sql.replace(/(on conflict \(source, incident_key\) where status <> )\?(\d+)/g, (_m, prefix, n) => {
    if (args[Number(n) - 1].value !== "closed") throw new Error("invalid incident conflict predicate");
    return `${prefix}'closed'`;
  });
  if (/::|\b(?:public|alerts)\./.test(sql)) throw new Error("unsupported store SQL dialect");
  // Remove now-unused parameter slots and renumber after the fixed predicate.
  const used = [...new Set([...sql.matchAll(/\?(\d+)/g)].map((m) => Number(m[1])))].sort((a, b) => a - b);
  const compact = used.map((n) => args[n - 1]);
  sql = sql.replace(/\?(\d+)/g, (_m, n) => `?${used.indexOf(Number(n)) + 1}`);
  return { sql, args: compact };
}

const JSON_COLUMNS = new Set([
  "classes",
  "quiet",
  "detail",
  "payload",
  "author",
  "linked_identity",
  "checkpoint",
  "seen_ids",
  "source_status",
  "value",
  "document",
  "aa",
  "batches",
  "prices",
]);
export function decodeStoreRow(row: Record<string, unknown>): Record<string, unknown> {
  return Object.fromEntries(
    Object.entries(row).map(([key, value]) => [
      key,
      JSON_COLUMNS.has(key) && typeof value === "string" ? JSON.parse(value) : value,
    ]),
  );
}

export function createTursoStoreClient(
  url: string,
  token: string,
  fetchFn: typeof fetch = fetch,
  writeMetrics?: TursoWriteMetrics,
): SqlClient {
  const endpoint = new URL(url.replace(/^libsql:/, "https:"));
  if (endpoint.protocol !== "https:" || endpoint.username || endpoint.password)
    throw new Error("invalid Turso store URL");
  return {
    ref: "turso:fleet-platform",
    async query<T = Record<string, unknown>>(input: string, writeContext?: TursoWriteContext): Promise<T[]> {
      const stmt = storeStatement(input);
      const res = await fetchFn(`${endpoint.href.replace(/\/$/, "")}/v2/pipeline`, {
        method: "POST",
        headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
        body: JSON.stringify({
          requests: [{ type: "execute", stmt: { ...stmt, want_rows: true } }, { type: "close" }],
        }),
        signal: AbortSignal.timeout(60_000),
      });
      if (!res.ok) throw new Error(`Turso store query failed (HTTP ${res.status})`);
      const out = (await res.json()) as any;
      const first = out?.results?.[0];
      const result = first?.response?.result;
      if (first?.type !== "ok" || !Array.isArray(result?.cols) || !Array.isArray(result?.rows)) {
        // Do not echo server SQL/errors: they can contain subscription keys.
        throw new Error("Turso store query failed or returned a malformed result");
      }
      const rows = result.rows.map((cells: any[]) => {
        if (cells.length !== result.cols.length) throw new Error("malformed Turso store row");
        const row = Object.fromEntries(
          cells.map((cell, i) => {
            let value: unknown;
            if (cell.type === "null") value = null;
            else if (cell.type === "text") value = cell.value;
            else if (cell.type === "integer" || cell.type === "float") {
              value = Number(cell.value);
              if (!Number.isFinite(value) || (cell.type === "integer" && !Number.isSafeInteger(value)))
                throw new Error("unsafe Turso numeric value");
            } else throw new Error("unsupported Turso store cell");
            return [result.cols[i].name, value];
          }),
        );
        return decodeStoreRow(row) as T;
      });
      if (writeMetrics && writeContext && first.response.type === "execute") {
        // Never expose SQL, bindings, rows or the rest of the response to the
        // meter. Observing a decoded success cannot change the query's result.
        try {
          writeMetrics.observe(writeContext, result.affected_row_count, result.rows_read, result.rows_written);
        } catch {
          // A diagnostic failure is not a failed write and must not induce retries.
        }
      }
      return rows;
    },
  };
}

export function createTursoStoreGetter(
  resolveSecret = resolveBwsSecret,
  writeMetrics?: TursoWriteMetrics,
): () => Promise<SqlClient | null> {
  let cached: Promise<SqlClient | null> | null = null;
  return () => {
    if (!cached) {
      const pending = (async () => {
        const token = await resolveSecret(DECISION_EVENTS_TURSO_TOKEN_NAME);
        return token ? createTursoStoreClient(DECISION_EVENTS_TURSO_URL, token, fetch, writeMetrics) : null;
      })();
      cached = pending;
      void pending.then(
        (v) => {
          if (!v && cached === pending) cached = null;
        },
        () => {
          if (cached === pending) cached = null;
        },
      );
    }
    return cached;
  };
}
export const getTursoStoreClient = createTursoStoreGetter();
