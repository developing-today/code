// Service credentials (src/fleet.js, #14373, design §4.8): the owner
// mints a credential directly for a platform service that is not an
// agent — the PM stack — with an addr (service:<its own name>) it
// sends as and the "read" scope for the owner's pipeline views. The
// PM console's edge Worker holds the value as a Worker secret and
// calls the bridge by service binding; these tests drive the same
// routes end to end through route(). Fleet mailboxes run the real
// fleetOp SQL on node:sqlite (as in fleet.test.mjs); credentials run
// on MemoryStore, which mirrors the Durable Object's hash-only
// semantics. A final block drives the sql helpers directly to prove
// hash-only storage with the addr column. All token values below are
// fixtures or values the Worker itself generated inside the test;
// nothing here is a real credential.
import assert from "node:assert/strict";
import { test } from "node:test";
import { DatabaseSync } from "node:sqlite";
import {
  fleetCall,
  fleetCredentialAuthSql,
  fleetCredentialPutSql,
  fleetOp,
  fleetSchema,
  fleetToken,
  mintFleetCredential,
} from "../src/fleet.js";
import { MemoryStore } from "../src/messages.js";
import { noteTokenHash } from "../src/notes.js";
import { route } from "../src/routes.js";

const TOKEN = "o".repeat(20) + "0123456789abcdefXYZ_-";
const CHATGPT = "c".repeat(40);
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
  const api = {
    mintCred: (body) => post(`/api/${TOKEN}/fleet/credentials/mint`, body),
    revokeCred: (body) => post(`/api/${TOKEN}/fleet/credentials/revoke`, body),
    agents: () => call(`/api/${TOKEN}/fleet/agents`),
    messages: (qs = {}) => call(`/api/${TOKEN}/fleet/messages?${new URLSearchParams(qs)}`),
    send: (body) => post(`/api/${TOKEN}/fleet/send`, body),
  };
  const as = (cred) => ({
    hello: (body) => post(`/fleet/${cred}/hello`, body),
    inbox: (qs) => call(`/fleet/${cred}/inbox?${new URLSearchParams(qs)}`),
    send: (body) => post(`/fleet/${cred}/send`, body),
    agents: () => call(`/fleet/${cred}/agents`),
    messages: (qs = {}) => call(`/fleet/${cred}/messages?${new URLSearchParams(qs)}`),
  });
  /** Mints the PM stack's service credential; returns the mint body. */
  const mintPm = async (body = { agent: "pm-stack", addr: "service:pm-stack" }) => {
    const res = await api.mintCred(body);
    assert.equal(res.status, 201, JSON.stringify(body));
    return res.json();
  };
  return { store, call, post, ft, api, as, mintPm };
}

test("mint: a service credential is owner-minted, value once, with send + read by default", async () => {
  const h = await harness();

  // Minting is owner-only: the fleet token and junk both 404.
  assert.equal((await h.post(`/api/${h.ft}/fleet/credentials/mint`, { agent: "pm-stack" })).status, 404);
  assert.equal((await h.post(`/api/nope/fleet/credentials/mint`, { agent: "pm-stack" })).status, 404);

  const mint = await h.mintPm();
  assert.match(mint.credential, VALUE);
  assert.equal(mint.agent, "pm-stack");
  assert.equal(mint.addr, "service:pm-stack");
  assert.deepEqual(mint.scopes, ["send", "read"]);

  // The addr must be service:<the credential's own name>.
  for (const body of [
    { agent: "pm-stack", addr: "service:other" },
    { agent: "pm-stack", addr: "agent:pm-stack" },
    { agent: "pm-stack", addr: "pm-stack" },
    { agent: "pm-stack", scopes: ["send", "root"] },
    { agent: "pm-stack", scopes: [] },
    { agent: "pm-stack", scopes: "send" },
  ]) {
    assert.equal((await h.api.mintCred(body)).status, 400, JSON.stringify(body));
  }
});

test("send: the PM credential delivers as service:pm-stack, told apart from owner and agent rows", async () => {
  const h = await harness();
  const mint = await h.mintPm();
  const pm = h.as(mint.credential);

  // Agents register the ordinary way, and one sends a row of its own.
  assert.equal((await h.post(`/fleet/${h.ft}/hello`, { agent: "felix-1", groups: ["muse"] })).status, 200);
  assert.equal((await h.post(`/fleet/${h.ft}/hello`, { agent: "other-1" })).status, 200);
  await h.post(`/fleet/${h.ft}/send`, { agent: "other-1", body: "agent row", to: "agent:felix-1" });
  // Central sends one with the owner token (from_addr null).
  await h.api.send({ body: "owner row", to: "agent:felix-1" });
  // The PM stack sends a message and a command with its credential.
  const sent = await pm.send({ agent: "pm-stack", body: "pm row", to: "agent:felix-1" });
  assert.equal(sent.status, 201, JSON.stringify(await sent.clone().json().catch(() => null)));
  const commanded = await pm.send({ agent: "pm-stack", body: "pm command", to: ["agent:felix-1"], kind: "command" });
  assert.equal(commanded.status, 201);

  const rows = (await (await h.api.messages({ agent: "felix-1", direction: "inbound", order: "asc" })).json()).messages;
  const byBody = Object.fromEntries(rows.map((m) => [m.body, m]));
  assert.equal(byBody["owner row"].from_addr, null);
  assert.equal(byBody["agent row"].from_addr, "agent:other-1");
  assert.equal(byBody["pm row"].from_addr, "service:pm-stack");
  assert.equal(byBody["pm row"].sender, "service:pm-stack");
  assert.equal(byBody["pm command"].from_addr, "service:pm-stack");
  assert.equal(byBody["pm command"].kind, "command");

  // The agent's own inbox view: the command names no sender, as every
  // command does; the message keeps the service's address.
  const inbox = (await (await h.call(`/fleet/${h.ft}/inbox?agent=felix-1&unread_only=false`)).json()).messages;
  const seen = Object.fromEntries(inbox.map((m) => [m.body, m]));
  assert.equal(seen["pm command"].from_addr, null);
  assert.equal(seen["pm command"].sender, null);
  assert.equal(seen["pm row"].from_addr, "service:pm-stack");
});

test("read: the pipeline views open to the read scope, and to nothing else", async () => {
  const h = await harness();
  const mint = await h.mintPm();
  const pm = h.as(mint.credential);
  await h.post(`/fleet/${h.ft}/hello`, { agent: "felix-1", groups: ["muse"] });
  await h.api.send({ body: "for felix", to: "agent:felix-1" });

  const agents = await pm.agents();
  assert.equal(agents.status, 200);
  assert.deepEqual(
    (await agents.json()).agents.map((a) => a.name),
    ["felix-1"],
  );
  const log = await pm.messages({ limit: "10", order: "desc" });
  assert.equal(log.status, 200);
  assert.equal((await log.json()).messages[0].body, "for felix");

  // A send-only service credential reads nothing.
  const sendOnly = await h.mintPm({ agent: "pm-stack", addr: "service:pm-stack", scopes: ["send"] });
  assert.equal((await h.as(sendOnly.credential).agents()).status, 404);
  assert.equal((await h.as(sendOnly.credential).messages()).status, 404);
  // The shared fleet token does not open the credential-side views.
  assert.equal((await h.as(h.ft).agents()).status, 404);
  // A credential with no read scope (an agent's own, via direct mint
  // without one) reads nothing either — and cannot hello as the service.
  const agentCred = await h.mintPm({ agent: "felix-1" });
  assert.deepEqual(agentCred.scopes, ["inbox", "send"]);
  assert.equal(agentCred.addr, null);
  assert.equal((await h.as(agentCred.credential).agents()).status, 404);
  assert.equal((await pm.hello({ agent: "pm-stack" })).status, 404, "no inbox scope: no mailbox of its own");
  assert.equal((await pm.send({ agent: "felix-1", body: "spoof" })).status, 404, "a credential sends only as itself");
});

test("revoke: revoking the PM credential stops its sends and reads at once, and nothing else", async () => {
  const h = await harness();
  const mint = await h.mintPm();
  const pm = h.as(mint.credential);
  await h.post(`/fleet/${h.ft}/hello`, { agent: "felix-1" });
  assert.equal((await pm.send({ agent: "pm-stack", body: "before", to: "agent:felix-1" })).status, 201);

  const revoked = await (await h.api.revokeCred({ agent: "pm-stack" })).json();
  assert.equal(revoked.revoked, 1);
  assert.equal((await pm.send({ agent: "pm-stack", body: "after", to: "agent:felix-1" })).status, 404);
  assert.equal((await pm.agents()).status, 404);
  assert.equal((await pm.messages()).status, 404);

  // The agent's mailbox and the shared token are untouched.
  const inbox = await h.call(`/fleet/${h.ft}/inbox?agent=felix-1&unread_only=false`);
  assert.equal(inbox.status, 200);
  assert.deepEqual(
    (await inbox.json()).messages.map((m) => m.body),
    ["before"],
  );
});

test("sql: the credential row stores the hash and the addr, never the value", async () => {
  const sql = sqlite();
  fleetSchema(sql);
  const store = {
    fleetCredentialPut: (rec) => fleetCredentialPutSql(sql, rec),
  };
  const mint = await mintFleetCredential(store, { agent: "pm-stack", addr: "service:pm-stack" }, "2026-11-24T00:00:00Z");
  const rows = sql.exec("SELECT * FROM fleet_credential").toArray();
  assert.equal(rows.length, 1);
  assert.equal(rows[0].token_hash, await noteTokenHash(mint.credential));
  assert.equal(rows[0].addr, "service:pm-stack");
  assert.deepEqual(JSON.parse(rows[0].scopes), ["send", "read"]);
  assert.ok(!JSON.stringify(rows).includes(mint.credential), "the value is nowhere in storage");
  const auth = fleetCredentialAuthSql(sql, await noteTokenHash(mint.credential));
  assert.deepEqual(auth, { agent: "pm-stack", scopes: ["send", "read"], addr: "service:pm-stack" });
});
