// The console's bridge views (src/fleet.js, #14374, design §4.9): the
// PM console's Messages and Agents surfaces read the unified pipeline,
// the unified agents list and per-agent credential state through the
// PM stack's service credential (the "read" scope, #14373), and the
// owner reads credential state at his own address. Oracles: a
// read-scope credential sees the pipeline with real senders (an
// agent-to-agent command names its agent sender), the directory marks
// a notes-only identity with no invented presence, credential state
// flips to revoked when the owner revokes and the list reflects it,
// no hash ever appears in a state row, and none of these views open
// to a credential without the read scope, to the shared fleet token,
// or to a revoke attempted at the credential address (revoking stays
// the owner's). Harness as in pipeline.test.mjs /
// service-credential.test.mjs: real fleetOp SQL on node:sqlite,
// MemoryStore for the rest. All token values are fixtures or values
// the Worker itself generated inside the test.
import assert from "node:assert/strict";
import { test } from "node:test";
import { DatabaseSync } from "node:sqlite";
import {
  fleetCall,
  fleetCredentialPutSql,
  fleetCredentialRevokeSql,
  fleetCredentialStateSql,
  fleetOp,
  fleetSchema,
  fleetToken,
} from "../src/fleet.js";
import { MemoryStore } from "../src/messages.js";
import { noteTokenHash } from "../src/notes.js";
import { route } from "../src/routes.js";

const TOKEN = "o".repeat(20) + "0123456789abcdefXYZ_-";
const DREW = "d".repeat(40);
const FELIX = "f".repeat(40);
const ARCHER = "a".repeat(40);
const SCOUT = "s".repeat(40);
const env = { ACCESS_TOKEN: TOKEN, MCP_TOKENS: JSON.stringify({ claude: "l".repeat(40), chatgpt: "c".repeat(40) }) };
const BASE = "https://bridge.example";

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
  const store = new MemoryStore(() => "2026-11-24T00:00:00.000Z");
  store.fleet = (op, args) => fleetCall(stub, op, args);
  const call = (path, init) => route(new Request(BASE + path, init), env, () => store);
  const post = (path, body, headers = {}) =>
    call(path, { method: "POST", headers: { "content-type": "application/json", ...headers }, body: JSON.stringify(body) });
  const bearer = (token) => ({ authorization: `Bearer ${token}` });
  const ft = await fleetToken(env);
  const as = (cred) => ({
    pipeline: (qs = {}) => call(`/fleet/${cred}/pipeline?${new URLSearchParams(qs)}`),
    directory: () => call(`/fleet/${cred}/directory`),
    credentials: () => call(`/fleet/${cred}/credentials`),
    offers: () => call(`/fleet/${cred}/offers`),
    post: (what, body) => post(`/fleet/${cred}/${what}`, body),
  });
  return {
    store,
    call,
    ft,
    as,
    register: (identity, token, extra = {}) => post("/notes/register", { identity, token, ...extra }, bearer(TOKEN)),
    noteSend: (token, to, note) => post("/notes/send", { to, note }, bearer(token)),
    fleetHello: (body) => post(`/fleet/${ft}/hello`, body),
    fleetSend: (body) => post(`/fleet/${ft}/send`, body),
    ownerSend: (body) => post(`/api/${TOKEN}/messages`, body),
    mintCred: (body) => post(`/api/${TOKEN}/fleet/credentials/mint`, body),
    revokeCred: (body) => post(`/api/${TOKEN}/fleet/credentials/revoke`, body),
    ownerCredentials: () => call(`/api/${TOKEN}/fleet/credentials`),
  };
}

/** Mints the PM stack's service credential; returns its value. */
async function mintPm(h) {
  const res = await h.mintCred({ agent: "pm-stack", addr: "service:pm-stack" });
  assert.equal(res.status, 201);
  return (await res.json()).credential;
}

/**
 * Rows on all three surfaces, plus credentials standing:
 * - felix and archer registered on fleet (hello) and notes; scout on
 *   notes only
 * - a fleet command from felix to archer (agent-to-agent)
 * - a connector instruction Drew sent to Claude
 * - a note from Drew to muse/felix
 * - a directly minted agent credential for felix (inbox + send)
 */
async function seeded() {
  const h = await harness();
  await h.register("user/drew", DREW);
  await h.register("muse/felix", FELIX);
  await h.register("muse/archer", ARCHER);
  await h.register("muse/scout", SCOUT);
  await h.fleetHello({ agent: "felix", groups: ["muse"] });
  await h.fleetHello({ agent: "archer", groups: ["muse"] });

  const sent = await h.fleetSend({ agent: "felix", body: "archer, do the thing", to: "agent:archer", kind: "command" });
  assert.equal(sent.status, 201, await sent.text().catch(() => ""));
  await h.ownerSend({ body: "owner instruction body", sender: "drew", to: "claude", kind: "instruction" });
  await h.noteSend(DREW, "muse/felix", "note for felix");

  const mint = await h.mintCred({ agent: "felix" });
  assert.equal(mint.status, 201);
  const felixCred = (await mint.json()).credential;
  const pm = await mintPm(h);
  return { ...h, felixCred, pm };
}

test("read scope: the pipeline through the PM credential shows every surface with its real sender", async () => {
  const h = await seeded();
  const res = await h.as(h.pm).pipeline({ limit: "200" });
  assert.equal(res.status, 200);
  const { messages } = await res.json();
  const byBody = new Map(messages.map((m) => [m.body, m]));

  const cmd = byBody.get("archer, do the thing");
  assert.equal(cmd.surface, "fleet");
  assert.equal(cmd.kind, "command");
  assert.equal(cmd.sender_addr, "agent:felix", "an agent-sent command names its real sender to the owner");
  assert.equal(cmd.receiver, "agent:archer");

  const ins = byBody.get("owner instruction body");
  assert.equal(ins.surface, "connector");
  assert.equal(ins.kind, "instruction");
  assert.equal(ins.sender_addr, "user:drew");

  const note = byBody.get("note for felix");
  assert.equal(note.surface, "notes");
  // A note's sender is its notes identity (user/drew), where a
  // connector row spells the same person user:drew: each surface's
  // own address, as the pipeline normalizes them.
  assert.equal(note.sender_addr, "user/drew");
  assert.equal(note.receiver, "muse/felix");

  // The same filters the console offers narrow it the same way.
  const only = await (await h.as(h.pm).pipeline({ sender: "felix", limit: "200" })).json();
  assert.ok(only.messages.length > 0);
  assert.ok(only.messages.every((m) => m.sender_addr === "agent:felix" || m.sender_addr === "muse/felix"));
});

test("read scope: the directory unifies the surfaces and invents no presence for notes", async () => {
  const h = await seeded();
  const res = await h.as(h.pm).directory();
  assert.equal(res.status, 200);
  const { agents } = await res.json();

  const scout = agents.filter((a) => a.name === "muse/scout");
  assert.equal(scout.length, 1, "a notes-only identity appears once");
  assert.equal(scout[0].surface, "notes");
  assert.equal(scout[0].presence, "none");
  assert.equal(scout[0].last_seen, null);

  const felix = agents.filter((a) => a.name === "felix" || a.name === "muse/felix");
  assert.deepEqual(
    felix.map((a) => a.surface).sort(),
    ["fleet", "notes"],
    "an agent on both surfaces has a row for each",
  );
  const fleetFelix = felix.find((a) => a.surface === "fleet");
  assert.deepEqual(fleetFelix.groups, ["muse"]);
  assert.equal(fleetFelix.presence, "last_seen");
});

test("credential state: the owner and the read scope see active, then revoked after the owner revokes", async () => {
  const h = await seeded();

  const before = await (await h.ownerCredentials()).json();
  const felix = before.credentials.find((c) => c.agent === "felix");
  assert.equal(felix.state, "active");
  assert.deepEqual(felix.scopes.sort(), ["inbox", "send"]);
  const pmRow = before.credentials.find((c) => c.agent === "pm-stack");
  assert.equal(pmRow.state, "active");
  assert.equal(pmRow.addr, "service:pm-stack");
  assert.deepEqual(pmRow.scopes.sort(), ["read", "send"]);
  // State only: no hash, no value, in either view.
  for (const view of [before, await (await h.as(h.pm).credentials()).json()]) {
    const text = JSON.stringify(view);
    assert.ok(!/token_hash|credential"/.test(text.replace(/"credentials"/g, "")), "no hash or value in the state view");
    assert.deepEqual(view.credentials.find((c) => c.agent === "felix").state, "active");
  }

  const revoked = await h.revokeCred({ agent: "felix" });
  assert.equal(revoked.status, 200);
  assert.equal((await revoked.json()).revoked, 1);

  const after = await (await h.as(h.pm).credentials()).json();
  const gone = after.credentials.find((c) => c.agent === "felix");
  assert.equal(gone.state, "revoked");
  assert.ok(gone.revoked_at, "the revocation time is shown");
  assert.deepEqual(gone.scopes, [], "a revoked agent carries no live scopes");
  // And the revoked credential itself is turned away everywhere.
  assert.equal((await h.as(h.felixCred).credentials()).status, 404);
});

test("no read scope, no views: an agent credential and the shared fleet token are turned away", async () => {
  const h = await seeded();
  for (const cred of [h.felixCred, h.ft]) {
    assert.equal((await h.as(cred).pipeline()).status, 404);
    assert.equal((await h.as(cred).directory()).status, 404);
    assert.equal((await h.as(cred).credentials()).status, 404);
  }
  // A wrong token is the same plain 404.
  assert.equal((await h.as("x".repeat(43)).pipeline()).status, 404);
});

test("a service credential can read state but never revoke: revoking stays the owner's route", async () => {
  const h = await seeded();
  // POST at the credential address is refused as a method, and there
  // is no revoke verb at that address at all.
  const post = await h.call(`/fleet/${h.pm}/credentials`, { method: "POST" });
  assert.equal(post.status, 405);
  assert.equal((await h.as(h.pm).post("revoke", { agent: "felix" })).status, 404);
  // The owner's state is untouched by the attempts.
  const state = await (await h.ownerCredentials()).json();
  assert.equal(state.credentials.find((c) => c.agent === "felix").state, "active");
});

test("sql: credential state folds live and revoked rows per agent, hashes never read out", async () => {
  const sql = sqlite();
  fleetSchema(sql);
  const store = {
    fleetCredentialPut: (rec) => fleetCredentialPutSql(sql, rec),
  };
  const one = await noteTokenHash("fixture-one");
  const two = await noteTokenHash("fixture-two");
  store.fleetCredentialPut({ token_hash: one, agent: "felix", scopes: ["inbox", "send"], created: "2026-11-24T00:00:00Z", addr: null });
  store.fleetCredentialPut({ token_hash: two, agent: "felix", scopes: ["inbox"], created: "2026-11-24T01:00:00Z", addr: null });

  let state = fleetCredentialStateSql(sql);
  assert.equal(state.length, 1);
  assert.equal(state[0].state, "active");
  assert.equal(state[0].credentials, 2);
  assert.equal(state[0].live, 2);
  assert.deepEqual(state[0].scopes.sort(), ["inbox", "send"]);
  assert.ok(!("token_hash" in state[0]));

  // Revoking one of two leaves the agent active; both, revoked.
  sql.exec("UPDATE fleet_credential SET revoked_at = ? WHERE token_hash = ?", "2026-11-24T02:00:00Z", one);
  state = fleetCredentialStateSql(sql);
  assert.equal(state[0].state, "active");
  assert.equal(state[0].revoked_at, null, "a part-revoked agent says no revocation time");
  assert.equal(fleetCredentialRevokeSql(sql, "felix", "2026-11-24T03:00:00Z"), 1);
  state = fleetCredentialStateSql(sql);
  assert.equal(state[0].state, "revoked");
  assert.equal(state[0].revoked_at, "2026-11-24T03:00:00Z");
  assert.deepEqual(state[0].scopes, []);
});

test("read scope: the offers list through the PM credential shows offers with their claims (#14375)", async () => {
  const h = await seeded();
  // The owner posts an offer; felix claims the one slot.
  const posted = await h.post(`/api/${TOKEN}/fleet/offer`, {
    body: "take the next bridge task",
    to: "agent:f*",
    kind: "command",
    slots: 1,
    sender: "drew",
  });
  assert.equal(posted.status, 201);
  const ownerList = await (await h.call(`/api/${TOKEN}/fleet/offers`)).json();
  assert.equal(ownerList.offers.length, 1);
  const offerId = ownerList.offers[0].id;
  const claim = await h.post(`/fleet/${h.ft}/claim`, { agent: "felix", offer_id: offerId });
  assert.equal(claim.status, 200, await claim.text().catch(() => ""));

  // The PM credential reads the same list, claims and claimant named.
  const res = await h.as(h.pm).offers();
  assert.equal(res.status, 200);
  const { offers } = await res.json();
  assert.equal(offers.length, 1);
  assert.equal(offers[0].id, offerId);
  assert.equal(offers[0].state, "filled");
  assert.equal(offers[0].claimed, 1);
  assert.equal(offers[0].claims.length, 1);
  assert.equal(offers[0].claims[0].agent, "felix");
  // The owner's list and the credential's list are the same projection.
  const after = await (await h.call(`/api/${TOKEN}/fleet/offers`)).json();
  assert.deepEqual(offers, after.offers);

  // A credential without the read scope does not open the list.
  assert.equal((await h.as(h.felixCred).offers()).status, 404);
  // The shared fleet token at the same address gets the agent view,
  // not the owner's: naming felix lists the offers felix may claim,
  // projected for an agent — no claims, no claimants.
  const agentView = await h.call(`/fleet/${h.ft}/offers?agent=felix`);
  assert.equal(agentView.status, 200);
  const agentOffers = (await agentView.json()).offers;
  assert.equal(agentOffers.length, 0, "the one offer is filled, so nothing stands open for felix");
});
