// State backup (#14951): the MessageStore export/import round trip,
// driven through the real Worker fetch and the real Durable Object
// (node:sqlite harness, as in do-store.test.mjs) with an in-memory
// R2. Oracle: a namespace restored from an export reads back the
// source's state — stats, identities and rows — and keeps issuing
// fresh ids above the restored high-water marks.
import assert from "node:assert/strict";
import { register } from "node:module";
import { test } from "node:test";
import { DatabaseSync } from "node:sqlite";

const cfWorkersStub =
  "data:text/javascript," +
  encodeURIComponent("export class DurableObject { constructor(ctx, env) { this.ctx = ctx; this.env = env; } }");
register(
  "data:text/javascript," +
    encodeURIComponent(`
      export async function resolve(specifier, context, nextResolve) {
        if (specifier === "cloudflare:workers") {
          return { url: ${JSON.stringify(cfWorkersStub)}, shortCircuit: true };
        }
        return nextResolve(specifier, context);
      }
    `),
  import.meta.url,
);

const { default: worker, MessageStore } = await import("../src/index.js");
const { importBridgeState } = await import("../src/backup.js");

const OWNER = "o".repeat(40);
const DREW = "d".repeat(40);
const ARCHER = "a".repeat(40);
const BASE = "https://bridge.example";

function sqlite() {
  const db = new DatabaseSync(":memory:");
  return {
    exec(q, ...args) {
      const out = db.prepare(q).all(...args);
      return { toArray: () => out, one: () => out[0] };
    },
  };
}

/** The BRIDGE_BACKUP binding's shape, in memory. */
function fakeR2() {
  const objects = new Map();
  return {
    objects,
    async put(key, body, opts = {}) {
      objects.set(key, { body: String(body), customMetadata: opts.customMetadata ?? {} });
    },
    async get(key) {
      const o = objects.get(key);
      return o ? { text: async () => o.body } : null;
    },
    async head(key) {
      const o = objects.get(key);
      return o ? { size: o.body.length, customMetadata: o.customMetadata } : null;
    },
  };
}

function harness(r2) {
  const object = new MessageStore({ storage: { sql: sqlite() }, waitUntil: () => {} }, {});
  const env = {
    ACCESS_TOKEN: <redacted>
    MESSAGES: { idFromName: (name) => name, get: () => object },
    ...(r2 ? { BRIDGE_BACKUP: r2 } : {}),
  };
  const ctx = { waitUntil: () => {} };
  const call = (path, init) => worker.fetch(new Request(BASE + path, init), env, ctx);
  const bearer = (token) => ({ authorization: `Bearer ${token}`, "content-type": "application/json" });
  return { object, env, ctx, call, bearer };
}

async function seed(h) {
  const reg = (identity, token) =>
    h.call("/notes/register", { method: "POST", headers: h.bearer(OWNER), body: JSON.stringify({ identity, token }) });
  assert.equal((await reg("user/drew", DREW)).status, 201);
  assert.equal((await reg("muse/archer", ARCHER)).status, 201);
  const note = await h.call("/notes/send", {
    method: "POST",
    headers: h.bearer(DREW),
    body: JSON.stringify({ to: "muse/archer", note: "back me up" }),
  });
  assert.equal(note.status, 201);
  const msg = await h.call(`/api/${OWNER}/messages`, {
    method: "POST",
    headers: h.bearer(OWNER),
    body: JSON.stringify({ body: "hello bridge", to: "claude" }),
  });
  assert.equal(msg.status, 201);
}

test("export/import restores the whole store into a fresh namespace", async () => {
  const r2 = fakeR2();
  const a = harness(r2);
  await seed(a);

  const exported = await a.call(`/api/${OWNER}/backup-export`, { method: "POST", headers: a.bearer(OWNER) });
  assert.equal(exported.status, 201);
  const summary = await exported.json();
  assert.ok(summary.rows > 0, "export carries rows");
  assert.equal(summary.tables.messages, 1);
  assert.ok(r2.objects.has("exports/latest.json"), "latest export landed in R2");

  const latest = await (await a.call(`/api/${OWNER}/backup-latest`, { headers: a.bearer(OWNER) })).json();
  assert.equal(latest.exported_at, summary.exported_at);
  assert.equal(latest.rows, summary.rows);

  // The scratch namespace: a fresh DO behind the same bucket.
  const b = harness(r2);
  assert.equal(b.object.stats("all").total, 0);
  const imported = await b.call(`/api/${OWNER}/backup-import`, {
    method: "POST",
    headers: b.bearer(OWNER),
    body: JSON.stringify({}),
  });
  assert.equal(imported.status, 200);
  assert.equal((await imported.json()).imported.messages, 1);

  // The restored namespace reads back the source's state.
  assert.deepEqual(b.object.stats("all"), a.object.stats("all"));
  assert.deepEqual(await b.object.noteIdentities(), await a.object.noteIdentities());
  const inbox = await (await b.call("/notes/inbox", { headers: b.bearer(ARCHER) })).json();
  assert.deepEqual(
    inbox.notes.map((n) => n.note),
    ["back me up"],
  );

  // Ids continue above the restored high-water marks, never reused.
  const before = b.object.stats("all").last_id;
  const added = b.object.add({ body: "after restore", sender: "test", direction: "inbound", source: "api", channel: "claude", kind: "message" });
  assert.ok(added.id > before, `new id ${added.id} above restored last_id ${before}`);
});

test("the scheduled handler writes the daily export", async () => {
  const r2 = fakeR2();
  const a = harness(r2);
  await seed(a);
  let pending;
  const ctx = {
    waitUntil: (p) => {
      pending = p;
    },
  };
  await worker.scheduled({}, a.env, ctx);
  await pending;
  assert.ok(r2.objects.has("exports/latest.json"), "cron run landed the latest export");
});

test("backup routes fail closed without the R2 binding, and stay owner-only", async () => {
  const unbound = harness(null);
  const res = await unbound.call(`/api/${OWNER}/backup-export`, { method: "POST", headers: unbound.bearer(OWNER) });
  assert.equal(res.status, 503);

  const r2 = fakeR2();
  const a = harness(r2);
  const wrong = await a.call(`/api/${"x".repeat(40)}/backup-latest`, { headers: a.bearer(OWNER) });
  assert.equal(wrong.status, 404);
});

test("import refuses a payload naming a table the store does not have", () => {
  const sql = sqlite();
  new MessageStore({ storage: { sql }, waitUntil: () => {} }, {});
  assert.throws(
    () =>
      importBridgeState(sql, {
        format: "message-bridge-backup/1",
        exported_at: new Date().toISOString(),
        tables: [{ name: "no_such_table", columns: ["x"], rows: [[1]] }],
        sequence: [],
      }),
    /does not have/,
  );
});
