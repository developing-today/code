// A Hrana v2 pipeline over node:sqlite (#13316): the kv door tests'
// stand-in for the shared Turso primary, so the catalogue's statements
// and migrations run against real SQLite.

import { DatabaseSync } from "node:sqlite";
import { KV_CATALOG, type KvPrimary, type KvValue } from "../src/kv-door.ts";

type Cell = KvValue;

function toSqlite(value: KvValue): unknown {
  switch (value.type) {
    case "null":
      return null;
    case "integer":
      return BigInt(value.value);
    case "float":
      return value.value;
    case "text":
      return value.value;
    case "blob":
      return Uint8Array.from(atob(value.base64), (c) => c.charCodeAt(0));
  }
}

function fromSqlite(value: unknown): Cell {
  if (value === null || value === undefined) return { type: "null" };
  if (typeof value === "bigint") return { type: "integer", value: value.toString() };
  if (typeof value === "number") {
    return Number.isInteger(value) ? { type: "integer", value: String(value) } : { type: "float", value };
  }
  if (typeof value === "string") return { type: "text", value };
  const bytes = value as Uint8Array;
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return { type: "blob", base64: btoa(bin) };
}

export class SqlitePrimary implements KvPrimary {
  readonly db = new DatabaseSync(":memory:");
  readonly bodies: unknown[] = [];

  private run(stmt: { sql: string; args?: KvValue[] }): {
    cols: { name: string }[];
    rows: Cell[][];
  } {
    const sql = stmt.sql.trim();
    if (/^(BEGIN|COMMIT|ROLLBACK)\b/i.test(sql)) {
      this.db.exec(sql);
      return { cols: [], rows: [] };
    }
    const prepared = this.db.prepare(sql);
    const records = prepared.all(...((stmt.args ?? []).map(toSqlite) as never[])) as Record<string, unknown>[];
    const names = records[0] ? Object.keys(records[0]) : [];
    return {
      cols: names.map((name) => ({ name })),
      rows: records.map((r) => Object.values(r).map(fromSqlite)),
    };
  }

  async pipeline(body: unknown): Promise<{ status: number; body: unknown }> {
    this.bodies.push(body);
    const results: unknown[] = [];
    for (const request of (body as { requests: Record<string, unknown>[] }).requests) {
      if (request.type === "close") {
        results.push({ type: "ok", response: { type: "close" } });
      } else if (request.type === "execute") {
        try {
          const result = this.run(request.stmt as { sql: string; args: KvValue[] });
          results.push({ type: "ok", response: { type: "execute", result } });
        } catch (e) {
          results.push({ type: "error", error: { message: String(e) } });
        }
      } else if (request.type === "batch") {
        const steps = (request.batch as { steps: Record<string, unknown>[] }).steps;
        const ok: boolean[] = [];
        const stepResults: unknown[] = [];
        const stepErrors: unknown[] = [];
        const holds = (cond: Record<string, unknown> | undefined): boolean => {
          if (!cond) return true;
          if (cond.type === "ok") return ok[cond.step as number] === true;
          if (cond.type === "not") return !holds(cond.cond as Record<string, unknown>);
          throw new Error("unsupported condition");
        };
        for (const step of steps) {
          if (!holds(step.condition as Record<string, unknown> | undefined)) {
            ok.push(false);
            stepResults.push(null);
            stepErrors.push(null);
            continue;
          }
          try {
            stepResults.push(this.run(step.stmt as { sql: string; args?: KvValue[] }));
            stepErrors.push(null);
            ok.push(true);
          } catch (e) {
            stepResults.push(null);
            stepErrors.push({ message: String(e) });
            ok.push(false);
          }
        }
        results.push({
          type: "ok",
          response: {
            type: "batch",
            result: { step_results: stepResults, step_errors: stepErrors },
          },
        });
      } else {
        results.push({
          type: "error",
          error: { message: "unsupported request" },
        });
      }
    }
    return { status: 200, body: { baton: null, base_url: null, results } };
  }

  migrationBatches(): number {
    return this.bodies.filter((b) => JSON.stringify(b).includes(KV_CATALOG.migrations.record)).length;
  }
}
