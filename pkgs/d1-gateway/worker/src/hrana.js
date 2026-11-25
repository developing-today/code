// Hrana-over-HTTP (the Turso/libSQL remote protocol, v2 and v3 JSON
// pipelines) served from a Cloudflare D1 database.
//
// What D1 can't do and how this maps it:
// - Interactive transactions. D1 has no BEGIN/COMMIT; it runs a batch
//   of statements atomically. A pipeline that sends BEGIN, statements,
//   COMMIT as separate execute requests, or a Hrana batch whose steps
//   are BEGIN / statements / conditional COMMIT / ROLLBACK (the shape
//   @libsql/client's batch(..., "write") and scripts/fleet_turso.py
//   send), runs its statements as one db.batch(): all or nothing.
// - Batons (a session that outlives one HTTP request). Every pipeline
//   is answered with baton null, so each request stands alone; a client
//   that keeps TEMP tables across requests does not work here.
// - Bare COMMIT/ROLLBACK/SAVEPOINT/RELEASE outside a BEGIN group are
//   no-ops (each D1 statement already commits on its own).

const MAX_SAFE = BigInt(Number.MAX_SAFE_INTEGER);
const MIN_SAFE = BigInt(Number.MIN_SAFE_INTEGER);

export function hranaToJs(v) {
  if (v === null || v === undefined) return null;
  switch (v.type) {
    case "null":
      return null;
    case "integer": {
      const n = BigInt(v.value);
      // D1 can't bind a BigInt; SQLite's INTEGER affinity turns the
      // decimal text back into an integer.
      return n <= MAX_SAFE && n >= MIN_SAFE ? Number(n) : String(n);
    }
    case "float":
      return typeof v.value === "number" ? v.value : Number(v.value);
    case "text":
      return v.value;
    case "blob":
      return base64ToBytes(v.base64 ?? "");
    default:
      throw new HranaError(`unsupported value type ${JSON.stringify(v.type)}`, "ARGS_INVALID");
  }
}

export function jsToHrana(v) {
  if (v === null || v === undefined) return { type: "null" };
  if (typeof v === "number") {
    return Number.isInteger(v) ? { type: "integer", value: String(v) } : { type: "float", value: v };
  }
  if (typeof v === "bigint") return { type: "integer", value: v.toString() };
  if (typeof v === "boolean") return { type: "integer", value: v ? "1" : "0" };
  if (typeof v === "string") return { type: "text", value: v };
  if (v instanceof ArrayBuffer) return { type: "blob", base64: bytesToBase64(new Uint8Array(v)) };
  if (ArrayBuffer.isView(v)) return { type: "blob", base64: bytesToBase64(new Uint8Array(v.buffer, v.byteOffset, v.byteLength)) };
  if (Array.isArray(v)) return { type: "blob", base64: bytesToBase64(Uint8Array.from(v)) };
  return { type: "text", value: String(v) };
}

function base64ToBytes(b64) {
  const s = atob(b64.replace(/-/g, "+").replace(/_/g, "/"));
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}

function bytesToBase64(bytes) {
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  }
  return btoa(s);
}

export class HranaError extends Error {
  constructor(message, code = "SQLITE_ERROR") {
    super(message);
    this.code = code;
  }
}

// Leading comments and whitespace removed, for keyword checks.
function head(sql) {
  let s = sql;
  for (;;) {
    const t = s.replace(/^\s+/, "");
    if (t.startsWith("--")) {
      const nl = t.indexOf("\n");
      s = nl < 0 ? "" : t.slice(nl + 1);
    } else if (t.startsWith("/*")) {
      const end = t.indexOf("*/");
      s = end < 0 ? "" : t.slice(end + 2);
    } else {
      return t;
    }
  }
}

export function controlKind(sql) {
  const m = /^(BEGIN|COMMIT|END|ROLLBACK|SAVEPOINT|RELEASE)\b/i.exec(head(sql));
  if (!m) return null;
  const k = m[1].toUpperCase();
  return k === "END" ? "COMMIT" : k;
}

export function returnsRows(sql) {
  const h = head(sql);
  const first = (/^[A-Za-z]+/.exec(h)?.[0] ?? "").toUpperCase();
  if (["SELECT", "VALUES", "PRAGMA", "EXPLAIN"].includes(first)) return true;
  const returning = /\breturning\b/i.test(h);
  if (first === "WITH") return returning || !/\b(insert|update|delete|replace)\b/i.test(h);
  return returning;
}

function isWrite(sql) {
  return /^(INSERT|UPDATE|DELETE|REPLACE|WITH)\b/i.test(head(sql));
}

function errorBody(e) {
  const message = String(e?.message ?? e);
  const m = /\b(SQLITE_[A-Z_]+)\b/.exec(message);
  let code = m ? m[1] : null;
  if (!code) code = typeof e?.code === "string" && !e.code.startsWith("ERR_") ? e.code : "SQLITE_ERROR";
  return { message, code };
}

function stmtSql(stmt, store) {
  if (typeof stmt?.sql === "string") return stmt.sql;
  if (stmt?.sql_id !== undefined && stmt?.sql_id !== null) {
    const sql = store.get(stmt.sql_id);
    if (sql === undefined) throw new HranaError(`unknown sql_id ${stmt.sql_id}`, "SQL_NOT_FOUND");
    return sql;
  }
  throw new HranaError("statement has no sql", "PROTO_ERROR");
}

function stmtArgs(stmt) {
  if (stmt?.named_args && stmt.named_args.length > 0) {
    throw new HranaError("named arguments are not supported by the D1 gateway", "ARGS_INVALID");
  }
  return (stmt?.args ?? []).map(hranaToJs);
}

function prepare(db, sql, args) {
  const p = db.prepare(sql);
  return args.length > 0 ? p.bind(...args) : p;
}

function emptyResult() {
  return { cols: [], rows: [], affected_row_count: 0, last_insert_rowid: null, rows_read: 0, rows_written: 0 };
}

// A D1Result (objects per row) as a Hrana StmtResult. Column order is
// the objects' key order, which is the statement's column order except
// for integer-like names; duplicate names collapse. Used for batches,
// where D1 has no raw() form.
function fromD1Result(sql, r) {
  const meta = r?.meta ?? {};
  const objs = r?.results ?? [];
  if (returnsRows(sql)) {
    const names = objs.length > 0 ? Object.keys(objs[0]) : [];
    return {
      cols: names.map((name) => ({ name, decltype: null })),
      rows: objs.map((o) => names.map((n) => jsToHrana(o[n]))),
      affected_row_count: isWrite(sql) ? objs.length : 0,
      last_insert_rowid: null,
      rows_read: meta.rows_read ?? 0,
      rows_written: meta.rows_written ?? 0,
    };
  }
  return {
    cols: [],
    rows: [],
    affected_row_count: meta.changes ?? 0,
    last_insert_rowid: meta.last_row_id === undefined || meta.last_row_id === null ? null : String(meta.last_row_id),
    rows_read: meta.rows_read ?? 0,
    rows_written: meta.rows_written ?? 0,
  };
}

export async function execOne(db, sql, args) {
  if (returnsRows(sql)) {
    const raw = await prepare(db, sql, args).raw({ columnNames: true });
    const [names = [], ...rows] = raw ?? [];
    return {
      cols: names.map((name) => ({ name, decltype: null })),
      rows: rows.map((row) => row.map(jsToHrana)),
      affected_row_count: isWrite(sql) ? rows.length : 0,
      last_insert_rowid: null,
      rows_read: 0,
      rows_written: 0,
    };
  }
  return fromD1Result(sql, await prepare(db, sql, args).run());
}

async function execAtomic(db, items) {
  if (items.length === 0) return [];
  const res = await db.batch(items.map(({ sql, args }) => prepare(db, sql, args)));
  return res.map((r, i) => fromD1Result(items[i].sql, r));
}

function evalCond(cond, results, errors) {
  if (cond === null || cond === undefined) return true;
  switch (cond.type) {
    case "ok":
      return results[cond.step] !== null && results[cond.step] !== undefined;
    case "error":
      return errors[cond.step] !== null && errors[cond.step] !== undefined;
    case "not":
      return !evalCond(cond.cond, results, errors);
    case "and":
      return (cond.conds ?? []).every((c) => evalCond(c, results, errors));
    case "or":
      return (cond.conds ?? []).some((c) => evalCond(c, results, errors));
    case "is_autocommit":
      return true;
    default:
      throw new HranaError(`unsupported condition ${JSON.stringify(cond.type)}`, "PROTO_ERROR");
  }
}

export async function execBatch(db, batch, store) {
  const steps = batch?.steps ?? [];
  const n = steps.length;
  const results = new Array(n).fill(null);
  const errors = new Array(n).fill(null);
  const sqls = steps.map((s) => stmtSql(s.stmt, store));
  const kinds = sqls.map(controlKind);
  if (kinds.includes("BEGIN")) {
    const body = [];
    for (let i = 0; i < n; i++) {
      if (kinds[i] === null) body.push(i);
    }
    let bodyResults = null;
    let bodyError = null;
    try {
      bodyResults = await execAtomic(
        db,
        body.map((i) => ({ sql: sqls[i], args: stmtArgs(steps[i].stmt) })),
      );
    } catch (e) {
      bodyError = errorBody(e);
    }
    const byStep = new Map(body.map((stepIdx, k) => [stepIdx, bodyResults ? bodyResults[k] : null]));
    for (let i = 0; i < n; i++) {
      if (!evalCond(steps[i].condition, results, errors)) continue;
      if (kinds[i] !== null) {
        results[i] = emptyResult();
      } else if (bodyError) {
        errors[i] = bodyError;
      } else {
        results[i] = byStep.get(i);
      }
    }
  } else {
    for (let i = 0; i < n; i++) {
      if (!evalCond(steps[i].condition, results, errors)) continue;
      if (kinds[i] !== null) {
        results[i] = emptyResult();
        continue;
      }
      try {
        results[i] = await execOne(db, sqls[i], stmtArgs(steps[i].stmt));
      } catch (e) {
        errors[i] = errorBody(e);
      }
    }
  }
  return { step_results: results, step_errors: errors };
}

// Split a script into statements on semicolons outside quotes and
// comments (enough for migration scripts sent as a "sequence").
export function splitStatements(sql) {
  const out = [];
  let cur = "";
  let i = 0;
  while (i < sql.length) {
    const c = sql[i];
    if (c === "'" || c === '"' || c === "`") {
      let j = i + 1;
      while (j < sql.length) {
        if (sql[j] === c) {
          if (sql[j + 1] === c) {
            j += 2;
            continue;
          }
          break;
        }
        j++;
      }
      cur += sql.slice(i, j + 1);
      i = j + 1;
    } else if (c === "-" && sql[i + 1] === "-") {
      const nl = sql.indexOf("\n", i);
      i = nl < 0 ? sql.length : nl;
    } else if (c === "/" && sql[i + 1] === "*") {
      const end = sql.indexOf("*/", i + 2);
      i = end < 0 ? sql.length : end + 2;
    } else if (c === ";") {
      // A trigger body holds its own semicolons; it ends at END;.
      if (/^CREATE\s+(?:TEMP\s+|TEMPORARY\s+)?TRIGGER\b/i.test(cur.trim()) && !/\bEND\s*$/i.test(cur)) {
        cur += c;
        i++;
        continue;
      }
      if (cur.trim()) out.push(cur.trim());
      cur = "";
      i++;
    } else {
      cur += c;
      i++;
    }
  }
  if (cur.trim()) out.push(cur.trim());
  return out;
}

function ok(response) {
  return { type: "ok", response };
}

function err(e) {
  return { type: "error", error: errorBody(e) };
}

// Answer one pipeline request body ({baton, requests}).
export async function pipeline(db, body, log = () => {}) {
  const reqs = Array.isArray(body?.requests) ? body.requests : [];
  const results = [];
  const store = new Map();
  let i = 0;
  while (i < reqs.length) {
    const r = reqs[i];
    try {
      if (r?.type === "execute") {
        const sql = stmtSql(r.stmt, store);
        const kind = controlKind(sql);
        if (kind === "BEGIN") {
          // Gather the executes up to the matching COMMIT/ROLLBACK and
          // run them as one atomic D1 batch.
          let j = i + 1;
          const group = [];
          let end = null;
          while (j < reqs.length && reqs[j]?.type === "execute") {
            const s = stmtSql(reqs[j].stmt, store);
            const k = controlKind(s);
            if (k === "COMMIT" || k === "ROLLBACK") {
              end = k;
              break;
            }
            if (k === null) group.push({ idx: j, sql: s, args: stmtArgs(reqs[j].stmt) });
            j++;
          }
          results.push(ok({ type: "execute", result: emptyResult() }));
          const groupOut = new Map();
          if (end === "ROLLBACK") {
            // Nothing is applied; reads still answer.
            for (const g of group) {
              if (returnsRows(g.sql) && !isWrite(g.sql)) {
                try {
                  groupOut.set(g.idx, ok({ type: "execute", result: await execOne(db, g.sql, g.args) }));
                } catch (e) {
                  groupOut.set(g.idx, err(e));
                }
              } else {
                groupOut.set(g.idx, ok({ type: "execute", result: emptyResult() }));
              }
            }
          } else {
            try {
              const out = await execAtomic(db, group);
              group.forEach((g, k) => groupOut.set(g.idx, ok({ type: "execute", result: out[k] })));
            } catch (e) {
              log("batch", e, group.map((g) => g.sql));
              for (const g of group) groupOut.set(g.idx, err(e));
            }
          }
          for (let k = i + 1; k < j; k++) {
            results.push(groupOut.get(k) ?? ok({ type: "execute", result: emptyResult() }));
          }
          if (end !== null) {
            const failed = end === "COMMIT" && group.some((g) => groupOut.get(g.idx)?.type === "error");
            results.push(
              failed
                ? err(new HranaError("cannot commit: a statement in the transaction failed", "SQLITE_ERROR"))
                : ok({ type: "execute", result: emptyResult() }),
            );
            i = j + 1;
          } else {
            i = j;
          }
          continue;
        }
        if (kind !== null) {
          results.push(ok({ type: "execute", result: emptyResult() }));
        } else {
          try {
            results.push(ok({ type: "execute", result: await execOne(db, sql, stmtArgs(r.stmt)) }));
          } catch (e) {
            log("execute", e, [sql]);
            results.push(err(e));
          }
        }
      } else if (r?.type === "batch") {
        results.push(ok({ type: "batch", result: await execBatch(db, r.batch, store) }));
      } else if (r?.type === "close") {
        results.push(ok({ type: "close" }));
      } else if (r?.type === "sequence") {
        const sql = typeof r.sql === "string" ? r.sql : store.get(r.sql_id);
        if (typeof sql !== "string") throw new HranaError("sequence has no sql", "PROTO_ERROR");
        for (const part of splitStatements(sql)) {
          if (controlKind(part) === null) await db.prepare(part).run();
        }
        results.push(ok({ type: "sequence" }));
      } else if (r?.type === "store_sql") {
        store.set(r.sql_id, r.sql);
        results.push(ok({ type: "store_sql" }));
      } else if (r?.type === "close_sql") {
        store.delete(r.sql_id);
        results.push(ok({ type: "close_sql" }));
      } else if (r?.type === "describe") {
        const sql = stmtSql(r, store);
        results.push(
          ok({
            type: "describe",
            result: { params: [], cols: [], is_explain: /^EXPLAIN\b/i.test(head(sql)), is_readonly: !isWrite(sql) && returnsRows(sql) },
          }),
        );
      } else if (r?.type === "get_autocommit") {
        results.push(ok({ type: "get_autocommit", is_autocommit: true }));
      } else {
        throw new HranaError(`unsupported request type ${JSON.stringify(r?.type)}`, "PROTO_ERROR");
      }
    } catch (e) {
      results.push(err(e));
    }
    i++;
  }
  return { baton: null, base_url: null, results };
}
