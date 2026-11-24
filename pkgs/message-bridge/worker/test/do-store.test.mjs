// The production store path (src/index.js): every request reaches the
// store through storeFor(env), a hand-written wrapper object over the
// MessageStore Durable Object stub. The rest of the suite drives
// route() with MemoryStore (messages.js), so a store method that the
// route handlers call but storeFor never wires up is invisible there —
// exactly how the DO's noteOrigin (notes reply routing, #14370) shipped
// unwrapped: replies worked on MemoryStore and threw through the DO.
//
// Oracle: this test drives the Worker's real default-export fetch —
// hence the real storeFor — with the real MessageStore class behind
// env.MESSAGES, its SQL on node:sqlite (the suite's DO sql harness, as
// in credentials.test.mjs). The only stub is the `cloudflare:workers`
// DurableObject base class, redirected by the module hook below. If a
// wrapper goes missing from storeFor again, the reply send below fails
// instead of returning 201.
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

const OWNER = "o".repeat(40);
const DREW = "d".repeat(40);
const ARCHER = "a".repeat(40);
const OTHER = "b".repeat(40);
const BASE = "https://bridge.example";

/** The Durable Object's sql.exec over node:sqlite (credentials.test.mjs). */
function sqlite() {
  const db = new DatabaseSync(":memory:");
  return {
    exec(q, ...args) {
      const out = db.prepare(q).all(...args);
      return { toArray: () => out, one: () => out[0] };
    },
  };
}

function harness() {
  const object = new MessageStore({ storage: { sql: sqlite() }, waitUntil: () => {} }, {});
  const env = {
    ACCESS_TOKEN: OWNER,
    MESSAGES: { idFromName: (name) => name, get: () => object },
  };
  const ctx = { waitUntil: () => {} };
  const call = (path, init) => worker.fetch(new Request(BASE + path, init), env, ctx);
  const bearer = (token) => ({ authorization: `Bearer ${token}`, "content-type": "application/json" });
  const registerIdentity = (identity, token) =>
    call("/notes/register", {
      method: "POST",
      headers: bearer(OWNER),
      body: JSON.stringify({ identity, token }),
    });
  const post = (token, body) =>
    call("/notes/send", { method: "POST", headers: bearer(token), body: JSON.stringify(body) });
  const inboxOf = async (token) => (await (await call("/notes/inbox", { headers: bearer(token) })).json()).notes;
  return { call, bearer, registerIdentity, post, inboxOf };
}

test("notes replies route to the hidden sender through the DO store (storeFor, #14370)", async () => {
  const { registerIdentity, post, inboxOf } = harness();
  assert.equal((await registerIdentity("user/drew", DREW)).status, 201);
  assert.equal((await registerIdentity("muse/archer", ARCHER)).status, 201);
  assert.equal((await registerIdentity("muse/other", OTHER)).status, 201);

  // An instruction from Drew lands in Archer's inbox sender-free, the
  // same projection MemoryStore gives (notes.test.mjs).
  const ins = await post(DREW, { to: "muse/archer", note: "run the check", kind: "instruction" });
  assert.equal(ins.status, 201);
  const archerInbox = await inboxOf(ARCHER);
  assert.deepEqual(
    archerInbox.map((n) => [n.kind, n.from, n.note, n.reply_to]),
    [["instruction", null, "run the check", null]],
  );
  const instructionId = archerInbox[0].id;

  // Archer replies naming only the note's id. The route resolves the
  // recipient store-side via store.noteOrigin — the wrapper this test
  // guards — and the reply lands in Drew's inbox, threaded.
  const reply = await post(ARCHER, { note: "check done", reply_to: instructionId });
  assert.equal(reply.status, 201);
  assert.deepEqual((await reply.json()).recipients, ["user/drew"]);
  const drewInbox = await inboxOf(DREW);
  assert.deepEqual(
    drewInbox.map((n) => [n.kind, n.from, n.note, n.reply_to]),
    [["note", "muse/archer", "check done", instructionId]],
  );

  // Replying to a note that is not in your inbox is refused through
  // the same store-side lookup (noteOrigin answers null).
  const foreign = await post(OTHER, { note: "me too", reply_to: instructionId });
  assert.equal(foreign.status, 400);
  assert.match((await foreign.json()).error, /not a note in your inbox/);
});
