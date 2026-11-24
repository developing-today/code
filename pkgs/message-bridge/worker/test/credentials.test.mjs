// Per-agent fleet credentials + bootstrap enroll tokens (src/fleet.js,
// #14368): routed end to end through route(). Fleet mailboxes run the
// real fleetOp SQL on node:sqlite (as in fleet.test.mjs); credentials,
// enroll tokens and notes run on MemoryStore, which mirrors the Durable
// Object's hash-only semantics. A second block drives the sql helpers
// directly to prove hash-only storage and the claim predicate.
// All token values below are fixtures or values the Worker itself
// generated inside the test; nothing here is a real credential.
import assert from "node:assert/strict";
import { test } from "node:test";
import { DatabaseSync } from "node:sqlite";
import {
  enrollTokenClaimSql,
  enrollTokenGetSql,
  enrollTokenMintSql,
  enrollTokenRevokeSql,
  exchangeEnrollToken,
  fleetCall,
  fleetCredentialAuthSql,
  fleetCredentialPutSql,
  fleetCredentialRevokeSql,
  fleetOp,
  fleetSchema,
  fleetToken,
  mintEnrollToken,
} from "../src/fleet.js";
import { MemoryStore } from "../src/messages.js";
import { noteTokenHash } from "../src/notes.js";
import { route } from "../src/routes.js";

const TOKEN = "o".repeat(20) + "0123456789abcdefXYZ_-";
const CHATGPT = "c".repeat(40);
const DREW_NOTES = "d".repeat(40); // fixture notes token for user/drew
const env = { ACCESS_TOKEN: TOKEN, CHATGPT_TOKEN: CHATGPT };
const BASE = "https://bridge.example";
const VALUE = /^[A-Za-z0-9_-]{43}$/;

/** The Durable Object's sql.exec over node:sqlite. */
function sqlite() {
  const db = new DatabaseSync(":memory:");
  return {
    exec(q, ...args) {
      const out = db.prepare(q).all(...args);
      return { toArray: () => out, one: () => out[0] };
    },
  };
}

async function harness() {
  const sql = sqlite();
  fleetSchema(sql);
  let clock = 0;
  const stub = { fleet: (op, args) => fleetOp(sql, op, args, `2026-11-24T00:00:${String(clock++).padStart(2, "0")}Z`) };
  const store = new MemoryStore();
  store.fleet = (op, args) => fleetCall(stub, op, args);
  const call = (path, init) => route(new Request(BASE + path, init), env, () => store);
  const ft = await fleetToken(env);
  const post = (path, body) =>
    call(path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  const bearer = (token) => ({ authorization: `Bearer ${token}`, "content-type": "application/json" });
  const api = {
    mint: (body) => post(`/api/${TOKEN}/enroll-tokens`, body),
    revokeEnroll: (body) => post(`/api/${TOKEN}/enroll-tokens/revoke`, body),
    revokeCred: (body) => post(`/api/${TOKEN}/fleet/credentials/revoke`, body),
    revokeNotes: (body) => post(`/api/${TOKEN}/notes-tokens/revoke`, body),
    agents: () => call(`/api/${TOKEN}/fleet/agents`),
    messages: (qs = {}) => call(`/api/${TOKEN}/fleet/messages?${new URLSearchParams(qs)}`),
    send: (body) => post(`/api/${TOKEN}/fleet/send`, body),
  };
  const exchange = (bootstrap_token, agent) => post(`/enroll/exchange`, { bootstrap_token, agent });
  const as = (cred) => ({
    hello: (body) => post(`/fleet/${cred}/hello`, body),
    inbox: (qs) => call(`/fleet/${cred}/inbox?${new URLSearchParams(qs)}`),
    ack: (body) => post(`/fleet/${cred}/ack`, body),
    send: (body) => post(`/fleet/${cred}/send`, body),
  });
  /** Mints and exchanges in one go; returns the exchange body. */
  const enroll = async (mintBody) => {
    const mint = await api.mint(mintBody);
    assert.equal(mint.status, 201, JSON.stringify(mintBody));
    const m = await mint.json();
    const ex = await exchange(m.bootstrap_token, mintBody.agent);
    assert.equal(ex.status, 201, `exchange for ${mintBody.agent}`);
    return { mint: m, creds: await ex.json() };
  };
  return { store, call, post, bearer, ft, api, exchange, as, enroll };
}

test("exchange: a bootstrap token provisions working fleet + notes credentials, once", async () => {
  const h = await harness();

  // Minting is owner-only: the fleet token and junk both 404.
  assert.equal((await h.post(`/api/${h.ft}/enroll-tokens`, { agent: "felix-1" })).status, 404);
  assert.equal((await h.post(`/api/nope/enroll-tokens`, { agent: "felix-1" })).status, 404);

  const mintRes = await h.api.mint({ agent: "Felix-1", kind: "muse", groups: ["muse"], notes_identity: "muse/felix-1" });
  assert.equal(mintRes.status, 201);
  const mint = await mintRes.json();
  assert.match(mint.bootstrap_token, VALUE);
  assert.equal(mint.agent, "felix-1", "names normalize at mint time");
  assert.deepEqual(mint.groups, ["muse"]);
  assert.ok(mint.expires_at > new Date().toISOString());

  // A name that isn't the minted name is refused — and does not burn the token.
  assert.equal((await h.exchange(mint.bootstrap_token, "other-1")).status, 404);
  assert.equal((await h.exchange("z".repeat(43), "felix-1")).status, 404, "unknown token");

  const exRes = await h.exchange(mint.bootstrap_token, "felix-1");
  assert.equal(exRes.status, 201);
  const creds = await exRes.json();
  assert.match(creds.fleet_token, VALUE);
  assert.match(creds.notes_token, VALUE);
  assert.notEqual(creds.fleet_token, creds.notes_token);
  assert.equal(creds.kind, "muse");
  assert.deepEqual(creds.groups, ["muse"]);

  // Single use: the same bootstrap token never works twice.
  assert.equal((await h.exchange(mint.bootstrap_token, "felix-1")).status, 404);

  // The exchange registered the mailbox with the bound groups.
  const agents = (await (await h.api.agents()).json()).agents;
  assert.deepEqual(
    agents.map((a) => [a.name, a.groups]),
    [["felix-1", ["muse"]]],
  );

  // The returned fleet credential works: hello, inbox, ack for its own name.
  const cred = h.as(creds.fleet_token);
  assert.equal((await cred.hello({ agent: "felix-1" })).status, 200);
  await h.api.send({ body: "first task", to: "agent:felix-1", kind: "command" });
  const inbox = await (await cred.inbox({ agent: "felix-1" })).json();
  assert.equal(inbox.messages.length, 1);
  assert.equal(inbox.messages[0].body, "first task");
  assert.equal(inbox.messages[0].sender, null, "commands stay sender-stripped");
  const ack = await cred.ack({ agent: "felix-1", ids: [inbox.messages[0].id] });
  assert.equal((await ack.json()).acked, 1);

  // The returned notes token opens that identity's inbox.
  await h.call("/notes/register", {
    method: "POST",
    headers: h.bearer(TOKEN),
    body: JSON.stringify({ identity: "user/drew", token: DREW_NOTES }),
  });
  const noteSend = await h.call("/notes/send", {
    method: "POST",
    headers: h.bearer(DREW_NOTES),
    body: JSON.stringify({ to: "muse/felix-1", note: "welcome aboard" }),
  });
  assert.equal(noteSend.status, 201);
  const notes = await (await h.call("/notes/inbox", { headers: h.bearer(creds.notes_token) })).json();
  assert.equal(notes.identity, "muse/felix-1");
  assert.deepEqual(
    notes.notes.map((n) => n.note),
    ["welcome aboard"],
  );

  // Without a notes identity at mint time, no notes token comes back.
  const plain = await h.enroll({ agent: "plain-1" });
  assert.equal(plain.creds.notes_token, null);
  assert.equal(plain.creds.notes_identity, null);
});

test("a credential acts only as its own agent, and cannot forge a sender", async () => {
  const h = await harness();
  const a = await h.enroll({ agent: "felix-1", groups: ["muse"] });
  const b = await h.enroll({ agent: "other-1" });
  const credA = h.as(a.creds.fleet_token);
  const credB = h.as(b.creds.fleet_token);

  await h.api.send({ body: "for other only", to: "agent:other-1" });

  // Every route naming the other agent is a plain 404 for A's credential.
  assert.equal((await credA.inbox({ agent: "other-1" })).status, 404);
  assert.equal((await credA.hello({ agent: "other-1" })).status, 404);
  assert.equal((await credA.ack({ agent: "other-1", ids: [1] })).status, 404);
  assert.equal((await credA.send({ agent: "other-1", body: "spoof" })).status, 404);
  // ...while B reads its own inbox fine, and A never saw the row.
  assert.equal((await (await credB.inbox({ agent: "other-1" })).json()).messages.length, 1);
  assert.equal((await (await credA.inbox({ agent: "felix-1" })).json()).messages.length, 0);

  // A sends to B claiming to be Drew: the row is stamped as A regardless.
  const sent = await credA.send({ agent: "felix-1", body: "hello from a", to: "agent:other-1", sender: "drew" });
  assert.equal(sent.status, 201);
  const rows = (await (await h.api.messages({ agent: "other-1", direction: "inbound" })).json()).messages;
  const fromA = rows.find((m) => m.body === "hello from a");
  assert.equal(fromA.from_addr, "agent:felix-1");
  assert.equal(fromA.sender, "felix-1");

  // A credential opens no owner route.
  assert.equal((await h.call(`/api/${a.creds.fleet_token}/fleet/agents`)).status, 404);
  assert.equal((await h.post(`/api/${a.creds.fleet_token}/enroll-tokens`, { agent: "x-1" })).status, 404);
});

test("scopes gate a credential's routes", async () => {
  const h = await harness();
  await h.enroll({ agent: "scoped-1" });
  const inboxOnly = `fixture-inbox-only-${"i".repeat(24)}`;
  const sendOnly = `fixture-send-only-${"s".repeat(26)}`;
  await h.store.fleetCredentialPut({
    token_hash: await noteTokenHash(inboxOnly),
    agent: "scoped-1",
    scopes: ["inbox"],
    created: new Date().toISOString(),
  });
  await h.store.fleetCredentialPut({
    token_hash: await noteTokenHash(sendOnly),
    agent: "scoped-1",
    scopes: ["send"],
    created: new Date().toISOString(),
  });
  assert.equal((await h.as(inboxOnly).inbox({ agent: "scoped-1" })).status, 200);
  assert.equal((await h.as(inboxOnly).send({ agent: "scoped-1", body: "x" })).status, 404, "no send scope");
  assert.equal((await h.as(sendOnly).inbox({ agent: "scoped-1" })).status, 404, "no inbox scope");
  assert.equal((await h.as(sendOnly).hello({ agent: "scoped-1" })).status, 404, "hello rides the inbox scope");
  assert.equal((await h.as(sendOnly).send({ agent: "scoped-1", body: "report" })).status, 201);
});

test("revoking a credential or notes token stops it at once; the shared token is unaffected", async () => {
  const h = await harness();
  const a = await h.enroll({ agent: "felix-1", notes_identity: "muse/felix-1" });
  const b = await h.enroll({ agent: "other-1" });
  const credA = h.as(a.creds.fleet_token);
  assert.equal((await credA.hello({ agent: "felix-1" })).status, 200);

  const rev = await h.api.revokeCred({ agent: "felix-1" });
  assert.deepEqual(await rev.json(), { agent: "felix-1", revoked: 1 });
  assert.equal((await credA.hello({ agent: "felix-1" })).status, 404);
  assert.equal((await credA.inbox({ agent: "felix-1" })).status, 404);
  assert.equal((await credA.send({ agent: "felix-1", body: "x" })).status, 404);
  assert.deepEqual(await (await h.api.revokeCred({ agent: "felix-1" })).json(), { agent: "felix-1", revoked: 0 });

  // Other agents and the shared fleet token keep working.
  assert.equal((await h.as(b.creds.fleet_token).hello({ agent: "other-1" })).status, 200);
  assert.equal((await h.as(h.ft).inbox({ agent: "felix-1" })).status, 200, "shared token still reads any mailbox");

  // Notes token revocation is by identity and just as immediate.
  assert.equal((await h.call("/notes/inbox", { headers: h.bearer(a.creds.notes_token) })).status, 200);
  const rn = await h.api.revokeNotes({ identity: "muse/felix-1" });
  assert.deepEqual(await rn.json(), { identity: "muse/felix-1", revoked: 1 });
  assert.equal((await h.call("/notes/inbox", { headers: h.bearer(a.creds.notes_token) })).status, 404);
});

test("bootstrap tokens expire and can be revoked while unused", async () => {
  const h = await harness();

  // Expiry: backdate the stored row (hash-keyed) past its deadline.
  const m1 = await (await h.api.mint({ agent: "late-1", ttl_seconds: 60 })).json();
  const hash1 = await noteTokenHash(m1.bootstrap_token);
  h.store.enrollTokens.get(hash1).expires_at = "2000-01-01T00:00:00.000Z";
  assert.equal((await h.exchange(m1.bootstrap_token, "late-1")).status, 404);

  // Revocation by value, then by hash; a used token can't be revoked.
  const m2 = await (await h.api.mint({ agent: "rev-1" })).json();
  assert.deepEqual(await (await h.api.revokeEnroll({ bootstrap_token: m2.bootstrap_token })).json(), { revoked: true });
  assert.equal((await h.exchange(m2.bootstrap_token, "rev-1")).status, 404);
  assert.deepEqual(await (await h.api.revokeEnroll({ bootstrap_token: m2.bootstrap_token })).json(), { revoked: false });

  const m3 = await (await h.api.mint({ agent: "rev-2" })).json();
  const hash3 = await noteTokenHash(m3.bootstrap_token);
  assert.deepEqual(await (await h.api.revokeEnroll({ token_hash: hash3 })).json(), { revoked: true });

  const used = await h.enroll({ agent: "used-1" });
  assert.deepEqual(await (await h.api.revokeEnroll({ bootstrap_token: used.mint.bootstrap_token })).json(), {
    revoked: false,
  });

  // Mint input is checked.
  assert.equal((await h.api.mint({ agent: "bad name" })).status, 400);
  assert.equal((await h.api.mint({ agent: "ok-1", ttl_seconds: 10 })).status, 400);
  assert.equal((await h.api.mint({ agent: "ok-1", notes_identity: "not-an-identity" })).status, 400);
});

test("sql store: hash-only storage and the claim predicate", async () => {
  const sql = sqlite();
  fleetSchema(sql);
  fleetSchema(sql); // idempotent
  const noteRegs = [];
  const store = {
    enrollTokenMint: (row) => enrollTokenMintSql(sql, row),
    enrollTokenGet: (hash) => enrollTokenGetSql(sql, hash),
    enrollTokenClaim: (hash, agent, now) => enrollTokenClaimSql(sql, hash, agent, now),
    enrollTokenRevoke: (hash) => enrollTokenRevokeSql(sql, hash, new Date().toISOString()),
    fleetCredentialPut: (rec) => fleetCredentialPutSql(sql, rec),
    fleetCredentialAuth: (hash) => fleetCredentialAuthSql(sql, hash),
    fleetCredentialRevoke: (agent) => fleetCredentialRevokeSql(sql, agent, new Date().toISOString()),
    fleet: (op, args) => fleetOp(sql, op, args),
    noteRegister: (reg) => noteRegs.push(reg),
  };

  const mint = await mintEnrollToken(store, { agent: "sql-1", kind: "muse", groups: ["muse"], notes_identity: "muse/sql-1" });
  const row = enrollTokenGetSql(sql, await noteTokenHash(mint.bootstrap_token));
  assert.equal(row.agent, "sql-1");
  assert.equal(row.used_at, null);

  // Wrong name cannot claim, and the failed claim leaves the token usable.
  assert.equal(await exchangeEnrollToken(store, { bootstrap_token: mint.bootstrap_token, agent: "sql-2" }), null);
  const out = await exchangeEnrollToken(store, { bootstrap_token: mint.bootstrap_token, agent: "sql-1" });
  assert.ok(out);
  assert.equal(await exchangeEnrollToken(store, { bootstrap_token: mint.bootstrap_token, agent: "sql-1" }), null, "single use");
  assert.equal(enrollTokenRevokeSql(sql, await noteTokenHash(mint.bootstrap_token), new Date().toISOString()), false, "used tokens can't be revoked");

  // Hash-only: no stored row, in either table, contains any token value,
  // and the stored hashes are exactly the SHA-256 of the values.
  const credRows = sql.exec("SELECT * FROM fleet_credential").toArray();
  assert.equal(credRows.length, 1);
  assert.equal(credRows[0].token_hash, await noteTokenHash(out.fleet_token));
  assert.deepEqual(JSON.parse(credRows[0].scopes), ["inbox", "send"]);
  const dump = JSON.stringify([credRows, sql.exec("SELECT * FROM enroll_token").toArray(), noteRegs]);
  for (const value of [mint.bootstrap_token, out.fleet_token, out.notes_token]) {
    assert.ok(!dump.includes(value), "a token value must never reach the store");
  }
  assert.equal(noteRegs[0].token_hash, await noteTokenHash(out.notes_token), "notes registration is hash-only too");

  // The credential authenticates until revoked.
  assert.deepEqual(fleetCredentialAuthSql(sql, await noteTokenHash(out.fleet_token)), { agent: "sql-1", scopes: ["inbox", "send"], addr: null });
  assert.equal(fleetCredentialRevokeSql(sql, "sql-1", new Date().toISOString()), 1);
  assert.equal(fleetCredentialAuthSql(sql, await noteTokenHash(out.fleet_token)), null);

  // Expired tokens cannot be claimed.
  const stale = await mintEnrollToken(store, { agent: "sql-9", ttl_seconds: 60 }, Date.now() - 120_000);
  assert.equal(await exchangeEnrollToken(store, { bootstrap_token: stale.bootstrap_token, agent: "sql-9" }), null);
});
