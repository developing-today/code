/**
 * Minimal libSQL HTTP client (the /v2/pipeline route every fleet
 * Turso writer uses: the Edge Function mirror, fleet-fetch, the
 * queue broker's Turso backend). Statements use typed args, never
 * string interpolation.
 */
export type TursoArg =
  | { type: "null" }
  | { type: "text"; value: string }
  | { type: "integer"; value: string };

export const tArg = (v: string | null | undefined): TursoArg =>
  v === null || v === undefined ? { type: "null" } : { type: "text", value: v };
export const iArg = (v: number | null | undefined): TursoArg =>
  v === null || v === undefined || !Number.isFinite(v)
    ? { type: "null" }
    : { type: "integer", value: String(Math.trunc(v)) };

interface PipelineResult {
  type: string;
  error?: { message?: string };
  response?: {
    result?: {
      rows?: TursoArg[][];
      last_insert_rowid?: number;
    };
  };
}

export class Turso {
  constructor(
    private url: string,
    private token: string,
  ) {}

  /** Run statements in one pipeline; throws on any error result. */
  async batch(stmts: { sql: string; args?: TursoArg[] }[]): Promise<PipelineResult[]> {
    if (!this.url || !this.token) throw new Error("turso url/token unset");
    const res = await fetch(`${this.url}/v2/pipeline`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${this.token}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        requests: [
          ...stmts.map((s) => ({
            type: "execute",
            stmt: s.args ? { sql: s.sql, args: s.args } : { sql: s.sql },
          })),
          { type: "close" },
        ],
      }),
    });
    if (!res.ok) throw new Error(`turso pipeline failed: HTTP ${res.status}`);
    const out = (await res.json()) as { results?: PipelineResult[] };
    const failed = (out.results ?? []).find((r) => r.type === "error");
    if (failed) {
      throw new Error(`turso pipeline failed: ${failed.error?.message ?? "pipeline error"}`);
    }
    return out.results ?? [];
  }

  async execute(sql: string, args?: TursoArg[]): Promise<PipelineResult> {
    const results = await this.batch([{ sql, args }]);
    return results[0];
  }

  /** First column of first row as text, or null when no row. */
  async queryText(sql: string, args?: TursoArg[]): Promise<string | null> {
    const r = await this.execute(sql, args);
    const rows = r.response?.result?.rows ?? [];
    const first = rows[0]?.[0];
    if (!first || first.type === "null") return null;
    return "value" in first ? String(first.value) : null;
  }
}
