import assert from "node:assert/strict";
import { test } from "node:test";
import { controlKind, pipeline, returnsRows, splitStatements } from "../src/hrana.js";
import { authorized, bindingName, handle } from "../src/http.js";
import { FakeD1 } from "./fake-d1.mjs";

const T = (value) => ({ type: "text", value });
const I = (value) => ({ type: "integer", value: String(value) });
const exec = (sql, args = []) => ({ type: "execute", stmt: { sql, args } });

async function setup() {
  const db = new FakeD1();
  await pipeline(db, {
    requests: [exec("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, n REAL)")],
  });
  return db;
}

test("keyword classification", () => {
  assert.equal(controlKind("  begin immediate"), "BEGIN");
  assert.equal(controlKind("END"), "COMMIT");
  assert.equal(controlKind("-- hi\nCOMMIT"), "COMMIT");
  assert.equal(controlKind("SELECT 1"), null);
  assert.equal(returnsRows("select 1"), true);
  assert.equal(returnsRows("INSERT INTO t VALUES (1) RETURNING id"), true);
  assert.equal(returnsRows("INSERT INTO t VALUES (1)"), false);
  assert.equal(returnsRows("WITH x AS (SELECT 1) SELECT * FROM x"), true);
  assert.equal(returnsRows("WITH x AS (SELECT 1) INSERT INTO t SELECT * FROM x"), false);
  assert.deepEqual(splitStatements("a; b ';' ; -- c;\n d"), ["a", "b ';'", "d"]);
});

test("execute: insert, select with typed values, close", async () => {
  const db = await setup();
  const res = await pipeline(db, {
    requests: [
      exec("INSERT INTO t (name, n) VALUES (?, ?)", [T("a"), { type: "float", value: 1.5 }]),
      exec("SELECT id, name, n, NULL AS z FROM t"),
      { type: "close" },
    ],
  });
  assert.equal(res.baton, null);
  assert.equal(res.results[0].type, "ok");
  assert.equal(res.results[0].response.result.affected_row_count, 1);
  assert.equal(res.results[0].response.result.last_insert_rowid, "1");
  const sel = res.results[1].response.result;
  assert.deepEqual(sel.cols.map((c) => c.name), ["id", "name", "n", "z"]);
  assert.deepEqual(sel.rows[0], [I(1), T("a"), { type: "float", value: 1.5 }, { type: "null" }]);
  assert.deepEqual(res.results[2], { type: "ok", response: { type: "close" } });
});

test("errors are per statement and carry the sqlite code", async () => {
  const db = await setup();
  const res = await pipeline(db, {
    requests: [
      exec("INSERT INTO t (name) VALUES ('a')"),
      exec("INSERT INTO t (name) VALUES ('a')"),
      exec("SELECT count(*) AS c FROM t"),
    ],
  });
  assert.equal(res.results[1].type, "error");
  assert.match(res.results[1].error.message, /UNIQUE/);
  assert.equal(res.results[1].error.code, "SQLITE_CONSTRAINT");
  assert.deepEqual(res.results[2].response.result.rows[0], [I(1)]);
});

test("BEGIN..COMMIT in one pipeline is atomic", async () => {
  const db = await setup();
  const okRun = await pipeline(db, {
    requests: [
      exec("BEGIN IMMEDIATE"),
      exec("INSERT INTO t (name) VALUES (?)", [T("x")]),
      exec("INSERT INTO t (name) VALUES (?)", [T("y")]),
      exec("COMMIT"),
      { type: "close" },
    ],
  });
  assert.deepEqual(okRun.results.map((r) => r.type), ["ok", "ok", "ok", "ok", "ok"]);
  assert.equal(okRun.results[2].response.result.affected_row_count, 1);
  const bad = await pipeline(db, {
    requests: [
      exec("BEGIN IMMEDIATE"),
      exec("INSERT INTO t (name) VALUES (?)", [T("z")]),
      exec("INSERT INTO t (name) VALUES (?)", [T("x")]),
      exec("COMMIT"),
    ],
  });
  assert.equal(bad.results[0].type, "ok");
  assert.equal(bad.results[2].type, "error");
  assert.equal(bad.results[3].type, "error");
  const count = await pipeline(db, { requests: [exec("SELECT count(*) FROM t")] });
  assert.deepEqual(count.results[0].response.result.rows[0], [I(2)]);
});

// The shape @libsql/client batch(..., "write") and fleet_turso.py send.
function txBatch(stmts) {
  const steps = [{ stmt: { sql: "BEGIN IMMEDIATE" } }];
  stmts.forEach((s, i) => steps.push({ stmt: s, condition: { type: "ok", step: i } }));
  const last = steps.length - 1;
  steps.push({ stmt: { sql: "COMMIT" }, condition: { type: "ok", step: last } });
  const commit = steps.length - 1;
  steps.push({ stmt: { sql: "ROLLBACK" }, condition: { type: "not", cond: { type: "ok", step: commit } } });
  return { type: "batch", batch: { steps } };
}

test("batch with BEGIN/conditional COMMIT/ROLLBACK", async () => {
  const db = await setup();
  const good = await pipeline(db, {
    requests: [txBatch([{ sql: "INSERT INTO t (name) VALUES ('p')" }, { sql: "SELECT name FROM t" }])],
  });
  const r = good.results[0].response.result;
  assert.equal(r.step_errors.every((e) => e === null), true);
  assert.notEqual(r.step_results[3], null); // COMMIT ran
  assert.equal(r.step_results[4], null); // ROLLBACK skipped
  assert.deepEqual(r.step_results[2].rows, [[T("p")]]);
  const bad = await pipeline(db, {
    requests: [txBatch([{ sql: "INSERT INTO t (name) VALUES ('q')" }, { sql: "INSERT INTO t (name) VALUES ('p')" }])],
  });
  const b = bad.results[0].response.result;
  assert.notEqual(b.step_errors[1], null);
  assert.equal(b.step_results[3], null); // COMMIT skipped
  assert.notEqual(b.step_results[4], null); // ROLLBACK ran
  const count = await pipeline(db, { requests: [exec("SELECT count(*) FROM t")] });
  assert.deepEqual(count.results[0].response.result.rows[0], [I(1)]);
});

test("plain batch without BEGIN evaluates conditions step by step", async () => {
  const db = await setup();
  const res = await pipeline(db, {
    requests: [
      {
        type: "batch",
        batch: {
          steps: [
            { stmt: { sql: "INSERT INTO t (name) VALUES ('a')" } },
            { stmt: { sql: "INSERT INTO t (name) VALUES ('a')" } },
            { stmt: { sql: "SELECT 1" }, condition: { type: "error", step: 1 } },
            { stmt: { sql: "SELECT 2" }, condition: { type: "ok", step: 1 } },
          ],
        },
      },
    ],
  });
  const r = res.results[0].response.result;
  assert.notEqual(r.step_results[0], null);
  assert.notEqual(r.step_errors[1], null);
  assert.notEqual(r.step_results[2], null);
  assert.equal(r.step_results[3], null);
});

test("store_sql, sequence, RETURNING, blobs", async () => {
  const db = await setup();
  const res = await pipeline(db, {
    requests: [
      { type: "sequence", sql: "CREATE TABLE u (b BLOB); CREATE INDEX u_b ON u (b);" },
      { type: "store_sql", sql_id: 1, sql: "INSERT INTO u (b) VALUES (?) RETURNING rowid AS id, b" },
      { type: "execute", stmt: { sql_id: 1, args: [{ type: "blob", base64: "AAEC" }] } },
      { type: "close_sql", sql_id: 1 },
    ],
  });
  assert.deepEqual(res.results.map((r) => r.type), ["ok", "ok", "ok", "ok"]);
  const ret = res.results[2].response.result;
  assert.deepEqual(ret.cols.map((c) => c.name), ["id", "b"]);
  assert.deepEqual(ret.rows[0][1], { type: "blob", base64: "AAEC" });
  assert.equal(ret.affected_row_count, 1);
});

test("http: routing, version probes and token hashes", async () => {
  const db = await setup();
  const token = "secret-token-value";
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(token));
  const hex = [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
  const env = { DB_FLEET_QUEUE: db, TOKEN_SHA256: JSON.stringify({ "fleet-queue": [hex] }) };
  assert.equal(bindingName("fleet-queue"), "DB_FLEET_QUEUE");
  assert.equal(await authorized(env, "fleet-queue", `Bearer ${token}`), true);
  assert.equal(await authorized(env, "fleet-queue", "Bearer nope"), false);
  assert.equal(await authorized(env, "other", `Bearer ${token}`), false);
  const base = "https://gw.example/db/fleet-queue";
  assert.equal((await handle(new Request(`${base}/v2`), env)).status, 200);
  assert.equal((await handle(new Request(`${base}/v3`), env)).status, 404);
  assert.equal((await handle(new Request("https://gw.example/db/nope/v2/pipeline", { method: "POST" }), env)).status, 404);
  const unauth = await handle(new Request(`${base}/v2/pipeline`, { method: "POST", body: "{}" }), env);
  assert.equal(unauth.status, 401);
  const res = await handle(
    new Request(`${base}/v2/pipeline`, {
      method: "POST",
      headers: { authorization: `Bearer ${token}` },
      body: JSON.stringify({ requests: [exec("SELECT count(*) AS c FROM t"), { type: "close" }] }),
    }),
    env,
  );
  assert.equal(res.status, 200);
  const body = await res.json();
  assert.deepEqual(body.results[0].response.result.rows[0], [I(0)]);
  const health = await (await handle(new Request("https://gw.example/health"), env)).json();
  assert.deepEqual(health.databases, ["fleet-queue"]);
});
