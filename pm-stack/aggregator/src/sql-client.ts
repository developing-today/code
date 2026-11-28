// The SQL client contract shared by the remote stores (remote-stores.ts)
// and the alerts ledger (alerts.ts), plus the literal helpers they use to
// build statements. The only remote implementation is the Turso store
// client (turso-stores.ts), whose storeStatement() turns every literal
// produced here into a typed argument before it rewrites any syntax.
import type { TursoWriteContext } from "./turso-write-metrics.ts";

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

export interface SqlClient {
  ref: string;
  query<T = Record<string, unknown>>(sql: string, writeContext?: TursoWriteContext): Promise<T[]>;
}
