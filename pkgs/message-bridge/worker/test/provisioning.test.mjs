// The console provisioning reads (src/fleet.js, #14376, design §4.9):
// the owner — and, through the read scope, the PM stack's service
// credential — can list every minted bootstrap enroll token with its
// state: pending, used by the exchange, expired by the clock, or
// revoked by the owner, each with the time that closed it. Oracles:
// the list never carries a hash or a token value (the value was shown
// once at mint and exists nowhere on the bridge to show again); a
// token minted for one name shows as that name's, and the exchange
// still refuses every other name (the S3 oracle, exercised from the
// console's own path: mint → list → exchange); the views stay shut to
// a credential without the read scope, to the shared fleet token, and
// to a revoke attempted at the credential address. Harness as in
// credentials.test.mjs: real fleetOp SQL on node:sqlite, MemoryStore
// for the rest; a second block drives the Durable Object's SQL helpers
// directly for store parity. All token values are fixtures or values
// the Worker itself generated inside the test.
import assert from "node:assert/strict";
import { test } from "node:test";
import { DatabaseSync } from "node:sqlite";
import {
  enrollTokenClaimSql,
  enrollTokenListSql,
  enrollTokenMintSql,
  enrollTokenRevokeAgentSql,
  enrollTokenRevokeSql,
  fleetCall,
  fleetOp,
  fleetSchema,
  fleetToken,
  foldEnrollTokens,
  mintEnrollToken,
} from "../src/fleet.js";
import { MemoryStore } from "../src/messages.js";
import { route } from "../src/routes.js";

const TOKEN = "o".repeat(20) + "0123456789abcdefXYZ_-";
const CHATGPT = "c".repeat(40);
const env = { ACCESS_TOKEN: TOKEN, CHATGPT_TOKEN: CHATGPT };
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
  const store = new MemoryStore();
  store.fleet = (op, args) => fleetCall(stub, op, args);
  const call = (path, init) => route(new Request(BASE + path, init), env, () => store);
  const ft = await fleetToken(env);
  const post = (path, body) =>
    call(path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  const api = {
    mint: (body) => post(`/api/${TOKEN}/enroll-tokens`, body),
    revokeEnroll: (body) => post(`/api/${TOKEN}/enroll-tokens/revoke`, body),
    list: () => call(`/api/${TOKEN}/enroll-tokens`),
    mintCred: (body) => post(`/api/${TOKEN}/fleet/credentials/mint`, body),
  };
  const exchange = (bootstrap_token, agent) => post(`/enroll/exchange`, { bootstrap_token, agent });
  const as = (cred) => ({
    list: () => call(`/fleet/${cred}/enroll-tokens`),
    revokeEnroll: (body) => post(`/fleet/${cred}/enroll-tokens/revoke`, body),
  });
  return { store, call, post, ft, api, exchange, as };
}

/** Mints one token through the owner's route; returns the mint body. */
async function minted(h, body) {
  const res = await h.api.mint(body);
  assert.equal(res.status, 201, JSON.stringify(body));
  return res.json();
}

test("the list shows each minted token's state and closing time, and never a hash or a value", async () => {
  const h = await harness();
  const pending = await minted(h, { agent: "pend-1", kind: "muse", groups: ["muse"] });
  const used = await minted(h, { agent: "used-1" });
  const ex = await h.exchange(used.bootstrap_token, "used-1");
  assert.equal(ex.status, 201);
  const revoked = await minted(h, { agent: "rev-1" });
  // The console revokes an unused token by the hash it derives from
  // nothing: the revoke route takes the stored hash, which the list
  // never shows — the owner's own hand-off copy is hashed here the
  // way the route accepts it.
  const byValue = await h.api.revokeEnroll({ bootstrap_token: revoked.bootstrap_token });
  assert.equal((await byValue.json()).revoked, true);
  // A token whose TTL has lapsed: minted two minutes in the past with
  // a one-minute TTL, straight into the store the route reads.
  await mintEnrollToken(h.store, { agent: "old-1", ttl_seconds: 60 }, Date.now() - 120_000);

  const res = await h.api.list();
  assert.equal(res.status, 200);
  const text = await res.text();
  const { enroll_tokens } = JSON.parse(text);
  const byAgent = new Map(enroll_tokens.map((t) => [t.agent, t]));

  assert.equal(byAgent.get("pend-1").state, "pending");
  assert.equal(byAgent.get("pend-1").kind, "muse");
  assert.deepEqual(byAgent.get("pend-1").groups, ["muse"]);
  assert.equal(byAgent.get("pend-1").used_at, null);
  assert.equal(byAgent.get("pend-1").revoked_at, null);

  assert.equal(byAgent.get("used-1").state, "used");
  assert.ok(byAgent.get("used-1").used_at, "the exchange's time is the closing time");

  assert.equal(byAgent.get("rev-1").state, "revoked");
  assert.ok(byAgent.get("rev-1").revoked_at, "the revoke's time is the closing time");

  assert.equal(byAgent.get("old-1").state, "expired");
  assert.ok(byAgent.get("old-1").expires_at <= new Date().toISOString(), "the lapse time is the expiry itself");
  assert.equal(byAgent.get("old-1").used_at, null);
  assert.equal(byAgent.get("old-1").revoked_at, null);

  // No hash, no value: neither the stored hashes nor any minted value
  // appears anywhere in the answer, and no row carries a hash field.
  for (const t of enroll_tokens) assert.ok(!("token_hash" in t), "no hash field on a row");
  for (const m of [pending, used, revoked]) assert.ok(!text.includes(m.bootstrap_token), "no minted value in the list");
  assert.ok(!/[0-9a-f]{64}/.test(text), "no SHA-256-shaped string in the list");
});

test("the console path: a token minted for one name enrolls that name only, and the list follows it", async () => {
  const h = await harness();
  const mint = await minted(h, { agent: "console-1", kind: "muse", groups: ["muse"], notes_identity: "muse/console-1" });

  // Before the exchange the list shows the pending token, bound to
  // the name it was minted for.
  let rows = (await (await h.api.list()).json()).enroll_tokens;
  const row = rows.find((t) => t.agent === "console-1");
  assert.equal(row.state, "pending");
  assert.equal(row.notes_identity, "muse/console-1");

  // The exchange refuses any other name (the S3 oracle), and the
  // token stays pending for its own.
  assert.equal((await h.exchange(mint.bootstrap_token, "other-1")).status, 404);
  rows = (await (await h.api.list()).json()).enroll_tokens;
  assert.equal(rows.find((t) => t.agent === "console-1").state, "pending");

  // Its own name exchanges, and the list then shows it used.
  const ex = await h.exchange(mint.bootstrap_token, "console-1");
  assert.equal(ex.status, 201);
  const creds = await ex.json();
  assert.equal(creds.agent, "console-1");
  assert.ok(creds.fleet_token && creds.notes_token, "the hand-off credentials came back once");
  rows = (await (await h.api.list()).json()).enroll_tokens;
  const after = rows.find((t) => t.agent === "console-1");
  assert.equal(after.state, "used");
  assert.ok(after.used_at);
});

test("revoking by agent name takes back that agent's pending tokens and no others", async () => {
  const h = await harness();
  await minted(h, { agent: "multi-1" });
  await minted(h, { agent: "multi-1" });
  const used = await minted(h, { agent: "multi-1" });
  assert.equal((await h.exchange(used.bootstrap_token, "multi-1")).status, 201);
  await minted(h, { agent: "other-2" });
  // A lapsed token for the same agent is already dead: not counted,
  // and its state stays expired rather than becoming revoked.
  await mintEnrollToken(h.store, { agent: "multi-1", ttl_seconds: 60 }, Date.now() - 120_000);

  const res = await h.api.revokeEnroll({ agent: "multi-1" });
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), { agent: "multi-1", revoked: 2 });

  const rows = (await (await h.api.list()).json()).enroll_tokens;
  const mine = rows.filter((t) => t.agent === "multi-1");
  assert.equal(mine.filter((t) => t.state === "revoked").length, 2);
  assert.equal(mine.filter((t) => t.state === "used").length, 1, "the used token became a credential; it is not this revoke's");
  assert.equal(mine.filter((t) => t.state === "expired").length, 1);
  assert.equal(rows.find((t) => t.agent === "other-2").state, "pending");
  // And again: nothing left to take back.
  assert.deepEqual(await (await h.api.revokeEnroll({ agent: "multi-1" })).json(), { agent: "multi-1", revoked: 0 });
});

test("the read scope sees the same state list; plainer doors stay shut, and revoking stays the owner's", async () => {
  const h = await harness();
  await minted(h, { agent: "pend-1" });

  // The PM stack's service credential (direct mint, service addr).
  const pmRes = await h.api.mintCred({ agent: "pm-stack", addr: "service:pm-stack" });
  assert.equal(pmRes.status, 201);
  const pm = (await pmRes.json()).credential;
  const viaPm = await h.as(pm).list();
  assert.equal(viaPm.status, 200);
  const rows = (await viaPm.json()).enroll_tokens;
  assert.equal(rows.find((t) => t.agent === "pend-1")?.state, "pending");

  // An enroll-minted agent credential (inbox + send, no read) is refused.
  const agentMint = await minted(h, { agent: "plain-1" });
  const ex = await h.exchange(agentMint.bootstrap_token, "plain-1");
  const agentCred = (await ex.json()).fleet_token;
  assert.equal((await h.as(agentCred).list()).status, 404);

  // The shared fleet token and a junk token are refused too.
  assert.equal((await h.call(`/fleet/${h.ft}/enroll-tokens`)).status, 404);
  assert.equal((await h.call(`/fleet/${"x".repeat(43)}/enroll-tokens`)).status, 404);

  // Minting and revoking never open at the credential address: the
  // revoke path is no route there at all (404), and a mint POST meets
  // the read view's method refusal — 405, as on every console view
  // since #14374, whose method check precedes the credential check.
  assert.equal((await h.as(pm).revokeEnroll({ token_hash: "0".repeat(64) })).status, 404);
  assert.equal((await h.post(`/fleet/${pm}/enroll-tokens`, { agent: "x-1" })).status, 405);
});

test("the Durable Object's SQL folds the same states, hash-free (store parity)", async () => {
  const sql = sqlite();
  fleetSchema(sql);
  const t0 = "2026-11-24T00:00:00.000Z";
  const row = (hash, agent, created, expires_at) => ({
    token_hash: hash,
    agent,
    kind: "agent",
    groups: [],
    notes_identity: null,
    expires_at,
    created,
  });
  enrollTokenMintSql(sql, row("a".repeat(64), "pend-1", t0, "2026-11-25T00:00:00.000Z"));
  enrollTokenMintSql(sql, row("b".repeat(64), "used-1", t0, "2026-11-25T00:00:00.000Z"));
  enrollTokenClaimSql(sql, "b".repeat(64), "used-1", "2026-11-24T01:00:00.000Z");
  enrollTokenMintSql(sql, row("c".repeat(64), "rev-1", t0, "2026-11-25T00:00:00.000Z"));
  enrollTokenRevokeSql(sql, "c".repeat(64), "2026-11-24T02:00:00.000Z");
  enrollTokenMintSql(sql, row("d".repeat(64), "old-1", t0, "2026-11-24T00:30:00.000Z"));

  const folded = foldEnrollTokens(enrollTokenListSql(sql), "2026-11-24T03:00:00.000Z");
  const byAgent = new Map(folded.map((t) => [t.agent, t]));
  assert.equal(byAgent.get("pend-1").state, "pending");
  assert.equal(byAgent.get("used-1").state, "used");
  assert.equal(byAgent.get("used-1").used_at, "2026-11-24T01:00:00.000Z");
  assert.equal(byAgent.get("rev-1").state, "revoked");
  assert.equal(byAgent.get("rev-1").revoked_at, "2026-11-24T02:00:00.000Z");
  assert.equal(byAgent.get("old-1").state, "expired");
  for (const t of folded) assert.ok(!("token_hash" in t));
  // The hashes the test itself stored never come back out.
  const text = JSON.stringify(folded);
  for (const hash of ["a", "b", "c", "d"]) assert.ok(!text.includes(hash.repeat(64)));

  // The by-agent revoke, same semantics: the pending token only.
  assert.equal(enrollTokenRevokeAgentSql(sql, "pend-1", "2026-11-24T04:00:00.000Z"), 1);
  assert.equal(enrollTokenRevokeAgentSql(sql, "used-1", "2026-11-24T04:00:00.000Z"), 0);
  const after = new Map(foldEnrollTokens(enrollTokenListSql(sql), "2026-11-24T05:00:00.000Z").map((t) => [t.agent, t]));
  assert.equal(after.get("pend-1").state, "revoked");
  assert.equal(after.get("pend-1").revoked_at, "2026-11-24T04:00:00.000Z");
});
