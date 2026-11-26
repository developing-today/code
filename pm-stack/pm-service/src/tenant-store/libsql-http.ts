/**
 * Async tenant-store surface for the hosted read path (CF-5c,
 * #14969).
 *
 * The synchronous SqlDb (migrate.ts) is driven by bun:sqlite, which
 * a Worker cannot run, and the tenant plane is Turso/libSQL over
 * HTTP, which a synchronous interface cannot drive. This module is
 * the async twin: `AsyncSqlDb` is the one method the read path
 * needs (`all`), and `libsqlHttpDb` implements it against the
 * libSQL server protocol (Hrana v3 over HTTP — POST /v3/pipeline),
 * which is plain fetch and runs unchanged in a Worker.
 *
 * The adapter holds the tenant DB URL and token only as the
 * closure's configuration, supplied by the Worker entry from its
 * secrets. Nothing here reads, logs, or persists a token (E §2.3);
 * the read path issues SELECTs only, and this adapter is only ever
 * handed SELECTs by the read-api store layer.
 */

export type AsyncSqlValue = string | number | null;

export interface AsyncSqlDb {
  all<T>(sql: string, params?: readonly AsyncSqlValue[]): Promise<T[]>;
}

export interface LibsqlHttpOptions {
  /** Tenant DB base URL: `libsql://…` or `https://…` (a trailing
   * slash and an explicit `/v3/pipeline` suffix are tolerated). */
  url: string;
  /** Auth token for the tenant DB, from the caller's secrets.
   * Omitted for tokenless local endpoints (sqld in dev). */
  token?: string;
  /** Fetch seam for tests; defaults to the global fetch. */
  fetchImpl?: typeof fetch;
}

type HranaValue =
  | { type: "null" }
  | { type: "text"; value: string }
  | { type: "integer"; value: string }
  | { type: "float"; value: number }
  | { type: "blob"; value: string };

interface HranaResultSet {
  cols: { name: string | null }[];
  rows: HranaValue[][];
}

interface HranaPipelineResult {
  type: string;
  response?: { type: string; result?: HranaResultSet };
  error?: { message: string; code?: string };
}

function encodeValue(value: AsyncSqlValue): HranaValue {
  if (value === null) return { type: "null" };
  if (typeof value === "string") return { type: "text", value };
  if (Number.isInteger(value)) return { type: "integer", value: String(value) };
  return { type: "float", value };
}

function decodeValue(value: HranaValue): unknown {
  switch (value.type) {
    case "null":
      return null;
    case "text":
      return value.value;
    case "integer":
      // Hrana carries integers as decimal strings; the tenant
      // store's integers are counts/epochs well inside the safe
      // range, and bun:sqlite serves them as JS numbers — decode
      // to number so both adapters serve identical shapes.
      return Number(value.value);
    case "float":
      return Number(value.value);
    case "blob":
      return Uint8Array.from(atob(value.value), (ch) => ch.charCodeAt(0));
  }
}

function pipelineUrl(rawUrl: string): string {
  let url = rawUrl.replace(/\/+$/, "");
  if (url.startsWith("libsql://")) url = `https://${url.slice("libsql://".length)}`;
  if (url.endsWith("/v3/pipeline")) return url;
  return `${url}/v3/pipeline`;
}

/** An AsyncSqlDb over a libSQL/Turso HTTP endpoint. One pipeline
 * request per query (execute + close): the read path is low-volume
 * and request-scoped in the Worker, so no baton is kept between
 * calls. */
export function libsqlHttpDb(options: LibsqlHttpOptions): AsyncSqlDb {
  const endpoint = pipelineUrl(options.url);
  const fetchImpl = options.fetchImpl ?? fetch;
  return {
    async all<T>(sql: string, params: readonly AsyncSqlValue[] = []): Promise<T[]> {
      const res = await fetchImpl(endpoint, {
        method: "POST",
        headers: {
          "content-type": "application/json",
          ...(options.token ? { authorization: `Bearer ${options.token}` } : {}),
        },
        body: JSON.stringify({
          requests: [{ type: "execute", stmt: { sql, args: params.map(encodeValue) } }, { type: "close" }],
        }),
      });
      if (!res.ok) throw new Error(`libsql http ${res.status}: ${await res.text()}`);
      const body = (await res.json()) as { results?: HranaPipelineResult[] };
      const first = body.results?.[0];
      if (!first) throw new Error("libsql http: empty pipeline response");
      if (first.type === "error" || first.error) {
        throw new Error(`libsql http: ${first.error?.message ?? "pipeline error"}`);
      }
      const result = first.response?.result;
      if (!result) throw new Error("libsql http: pipeline response carried no result set");
      const names = result.cols.map((col) => col.name ?? "");
      return result.rows.map((row) => {
        const record: Record<string, unknown> = {};
        row.forEach((value, i) => {
          record[names[i] as string] = decodeValue(value);
        });
        return record as T;
      });
    },
  };
}
