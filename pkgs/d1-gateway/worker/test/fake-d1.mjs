// A small D1 stand-in over node:sqlite, with D1's shapes: prepare/bind,
// all/run/raw({columnNames}), batch (atomic), exec. D1 refuses
// BEGIN/COMMIT itself, and so does this fake.
import { DatabaseSync } from "node:sqlite";
import { returnsRows } from "../src/hrana.js";

function refuseControl(sql) {
  if (/^\s*(BEGIN|COMMIT|END|ROLLBACK|SAVEPOINT|RELEASE)\b/i.test(sql)) {
    throw new Error(`D1_ERROR: not authorized to use ${sql.trim().split(/\s+/)[0]}: SQLITE_AUTH`);
  }
}

const CODES = { 1: "ERROR", 19: "CONSTRAINT", 23: "AUTH" };
// D1's error text: "D1_ERROR: <sqlite message>: SQLITE_<CODE>".
function d1Error(e) {
  if (/^D1_ERROR/.test(e.message)) return e;
  const code = CODES[(e.errcode ?? 1) & 0xff] ?? "ERROR";
  return new Error(`D1_ERROR: ${e.errstr ? e.message : e.message}: SQLITE_${code}`);
}

class Stmt {
  constructor(db, sql, args = []) {
    this.db = db;
    this.sql = sql;
    this.args = args;
  }
  bind(...args) {
    if (args.length > 100) {
      throw new Error("D1_ERROR: variable number must be between ?1 and ?100 at offset 0: SQLITE_ERROR");
    }
    return new Stmt(this.db, this.sql, args);
  }
  #prep() {
    refuseControl(this.sql);
    const st = this.db.prepare(this.sql);
    st.setReadBigInts?.(false);
    return st;
  }
  #norm() {
    return this.args;
  }
  async all() {
    return this.allSync();
  }
  allSync() {
    try {
      return this.#allSync();
    } catch (e) {
      throw d1Error(e);
    }
  }
  #allSync() {
    const st = this.#prep();
    if (returnsRows(this.sql)) {
      const results = st.all(...this.#norm());
      return { success: true, results, meta: { changes: 0, last_row_id: null, rows_read: results.length, rows_written: 0 } };
    }
    const r = st.run(...this.#norm());
    return {
      success: true,
      results: [],
      meta: { changes: Number(r.changes), last_row_id: Number(r.lastInsertRowid), rows_read: 0, rows_written: Number(r.changes) },
    };
  }
  async run() {
    return this.allSync();
  }
  async raw(opts = {}) {
    try {
      return this.#raw(opts);
    } catch (e) {
      throw d1Error(e);
    }
  }
  #raw(opts) {
    const st = this.#prep();
    const objs = st.all(...this.#norm());
    const names = st.columns ? st.columns().map((c) => c.name) : objs.length > 0 ? Object.keys(objs[0]) : [];
    const rows = objs.map((o) => names.map((n) => o[n]));
    return opts.columnNames ? [names, ...rows] : rows;
  }
}

export class FakeD1 {
  constructor() {
    this.db = new DatabaseSync(":memory:");
  }
  prepare(sql) {
    return new Stmt(this.db, sql);
  }
  async batch(stmts) {
    this.db.exec("BEGIN");
    try {
      const out = stmts.map((s) => s.allSync());
      this.db.exec("COMMIT");
      return out;
    } catch (e) {
      this.db.exec("ROLLBACK");
      throw e;
    }
  }
  async exec(sql) {
    this.db.exec(sql);
    return { count: 1, duration: 0 };
  }
}
