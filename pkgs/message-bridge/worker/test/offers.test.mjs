// Offer pool (src/fleet.js, design §4.2, #14371): claim-style
// distribution routed end to end through route(), with the real
// fleetOp SQL running on node:sqlite in place of the Durable Object's
// storage (same harness as fleet.test.mjs), plus a controllable clock
// for the expiry cases.
import assert from "node:assert/strict";
import { test } from "node:test";
import { DatabaseSync } from "node:sqlite";
import { fleetCall, fleetOp, fleetSchema, fleetToken } from "../src/fleet.js";
import { MemoryStore } from "../src/messages.js";
import { noteTokenHash } from "../src/notes.js";
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
  let nowMs = Date.parse("2026-11-24T00:00:00.000Z");
  const now = () => new Date(nowMs).toISOString();
  const stub = { fleet: (op, args) => fleetOp(sql, op, args, now()) };
  const store = new MemoryStore(now);
  store.fleet = (op, args) => fleetCall(stub, op, args);
  const call = (path, init) => route(new Request(BASE + path, init), env, () => store);
  const ft = await fleetToken(env);
  const post = (path, body) =>
    call(path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  const agent = {
    hello: (body) => post(`/fleet/${ft}/hello`, body),
    inbox: (qs) => call(`/fleet/${ft}/inbox?${new URLSearchParams(qs)}`),
    send: (body) => post(`/fleet/${ft}/send`, body),
    offer: (body) => post(`/fleet/${ft}/offer`, body),
    offers: (qs) => call(`/fleet/${ft}/offers?${new URLSearchParams(qs)}`),
    claim: (body) => post(`/fleet/${ft}/claim`, body),
  };
  const owner = {
    send: (body) => post(`/api/${TOKEN}/fleet/send`, body),
    offer: (body) => post(`/api/${TOKEN}/fleet/offer`, body),
    offers: () => call(`/api/${TOKEN}/fleet/offers`),
    messages: (qs = {}) => call(`/api/${TOKEN}/fleet/messages?${new URLSearchParams(qs)}`),
    pipeline: (qs = {}) => call(`/api/${TOKEN}/pipeline?${new URLSearchParams(qs)}`),
  };
  return { store, sql, call, ft, post, agent, owner, advance: (secs) => (nowMs += secs * 1000) };
}

async function register(agent) {
  assert.equal((await agent.hello({ agent: "fable" })).status, 200);
  assert.equal((await agent.hello({ agent: "falcon", groups: ["crew"] })).status, 200);
  assert.equal((await agent.hello({ agent: "fennel" })).status, 200);
  assert.equal((await agent.hello({ agent: "muse-1", groups: ["crew"] })).status, 200);
}

test("slots 2, three eligible claimants: exactly the first two are delivered, the third is filled", async () => {
  const { agent, owner } = await harness();
  await register(agent);

  const res = await owner.offer({ body: "take the next task", to: "agent:f*", kind: "command", slots: 2, sender: "drew" });
  assert.equal(res.status, 201);
  const { offer } = await res.json();
  assert.equal(offer.state, "open");
  assert.equal(offer.claimed, 0);
  assert.equal(offer.remaining, 2);
  assert.deepEqual(offer.claims, []);

  // Stored once, not fanned out: no mailbox rows anywhere yet, but the
  // eligible agents see the offer in their inbox's offers section.
  for (const name of ["fable", "falcon", "fennel", "muse-1"]) {
    const box = await (await agent.inbox({ agent: name })).json();
    assert.deepEqual(box.messages, [], name);
  }
  assert.deepEqual((await (await agent.inbox({ agent: "fable" })).json()).offers.map((o) => o.id), [offer.id]);
  assert.deepEqual((await (await agent.offers({ agent: "falcon" })).json()).offers.map((o) => o.id), [offer.id]);
  assert.deepEqual((await (await agent.inbox({ agent: "muse-1" })).json()).offers, [], "outside the pattern: no offer shown");

  // First two claims win a normal mailbox row on the offer's batch.
  const c1 = await agent.claim({ agent: "fable", offer_id: offer.id });
  assert.equal(c1.status, 200);
  const claim1 = await c1.json();
  assert.equal(claim1.message.body, "take the next task");
  assert.equal(claim1.message.batch, offer.batch);
  assert.equal(claim1.remaining, 1);
  const c2 = await agent.claim({ agent: "falcon", offer_id: offer.id });
  assert.equal(c2.status, 200);
  const claim2 = await c2.json();
  assert.equal(claim2.remaining, 0);

  // The third eligible claim gets a definite "filled" — and no row.
  const c3 = await agent.claim({ agent: "fennel", offer_id: offer.id });
  assert.equal(c3.status, 400);
  assert.match((await c3.json()).error, /filled/);
  assert.deepEqual((await (await agent.inbox({ agent: "fennel" })).json()).messages, []);
  assert.deepEqual((await (await owner.messages({ agent: "fennel" })).json()).messages, []);

  // The owner's list names the claimants, their times and their rows.
  const list = (await (await owner.offers()).json()).offers;
  assert.equal(list.length, 1);
  assert.equal(list[0].state, "filled");
  assert.equal(list[0].claimed, 2);
  assert.deepEqual(
    list[0].claims.map((c) => [c.agent, c.message_id]),
    [
      ["fable", claim1.message.id],
      ["falcon", claim2.message.id],
    ],
  );
  assert.ok(list[0].claims.every((c) => typeof c.claimed_at === "string" && c.claimed_at.length > 0));
});

test("an agent outside the offer's pattern can neither see nor claim it", async () => {
  const { agent, owner } = await harness();
  await register(agent);
  const { offer } = await (await owner.offer({ body: "crew only", to: "group:crew", slots: 1 })).json();
  // fennel is registered but not in crew; ghost is not registered at all.
  assert.deepEqual((await (await agent.offers({ agent: "fennel" })).json()).offers, []);
  const denied = await agent.claim({ agent: "fennel", offer_id: offer.id });
  assert.equal(denied.status, 400);
  assert.match((await denied.json()).error, /not eligible/);
  const ghost = await agent.claim({ agent: "ghost", offer_id: offer.id });
  assert.equal(ghost.status, 400);
  assert.match((await ghost.json()).error, /not eligible/);
  // A crew member sees and claims it.
  assert.deepEqual((await (await agent.offers({ agent: "muse-1" })).json()).offers.map((o) => o.id), [offer.id]);
  assert.equal((await agent.claim({ agent: "muse-1", offer_id: offer.id })).status, 200);
});

test("a claimant's reply threads onto the offer batch, visible to the owner with the claimant named", async () => {
  const { agent, owner } = await harness();
  await register(agent);
  const { offer } = await (await owner.offer({ body: "do the thing", to: "agent:f*", slots: 1, sender: "drew" })).json();
  const claim = await (await agent.claim({ agent: "falcon", offer_id: offer.id })).json();

  const res = await agent.send({ agent: "falcon", body: "thing done", reply_to: claim.message.id });
  assert.equal(res.status, 201);
  const reply = (await res.json()).message;
  assert.equal(reply.batch, offer.batch);
  assert.equal(reply.agent, "falcon");

  // The pipeline filtered by the batch shows delivery and reply together.
  const feed = (await (await owner.pipeline({ batch: offer.batch })).json()).messages;
  assert.deepEqual(
    feed.map((m) => [m.surface, m.direction, m.receiver ?? m.sender_addr, m.body]),
    [
      ["fleet", "inbound", "agent:falcon", "do the thing"],
      ["fleet", "outbound", "user:drew", "thing done"],
    ],
  );
  const list = (await (await owner.offers()).json()).offers;
  assert.equal(list[0].claims[0].agent, "falcon");
  assert.equal(list[0].claims[0].message_id, claim.message.id);
});

test("an instruction offer reaches claimants sender-stripped; the owner sees the real sender", async () => {
  const { agent, owner } = await harness();
  await register(agent);

  // Owner-posted command offer: claim response and inbox show no sender.
  const { offer } = await (await owner.offer({ body: "quiet task", to: "agent:fable", kind: "command", slots: 1, sender: "drew" })).json();
  const seen = (await (await agent.offers({ agent: "fable" })).json()).offers[0];
  assert.equal(seen.sender, null);
  assert.equal(seen.from_addr, null);
  const claim = await (await agent.claim({ agent: "fable", offer_id: offer.id })).json();
  assert.equal(claim.message.sender, null);
  assert.equal(claim.message.from_addr, null);
  const box = (await (await agent.inbox({ agent: "fable", unread_only: "false" })).json()).messages;
  assert.equal(box[0].sender, null);

  // Agent-posted command offer: the owner list names the poster; the
  // claimer still sees no sender.
  const posted = await (await agent.offer({ agent: "muse-1", body: "agent task", to: "group:crew", kind: "command", slots: 1 })).json();
  assert.equal(posted.offer.sender_addr, "agent:muse-1");
  const list = (await (await owner.offers()).json()).offers;
  assert.equal(list.find((o) => o.id === posted.offer.id).sender_addr, "agent:muse-1");
  const claim2 = await (await agent.claim({ agent: "falcon", offer_id: posted.offer.id })).json();
  assert.equal(claim2.message.sender, null);
  assert.equal(claim2.message.from_addr, null);

  // A plain message offer keeps its sender, like any plain message.
  const { offer: msgOffer } = await (await owner.offer({ body: "fyi all", to: "all", slots: 1, sender: "drew" })).json();
  const claim3 = await (await agent.claim({ agent: "fennel", offer_id: msgOffer.id })).json();
  assert.equal(claim3.message.sender, "drew");
});

test("concurrent claims cannot oversubscribe the slots", async () => {
  const { agent, owner } = await harness();
  await register(agent);
  await agent.hello({ agent: "fiona" });
  await agent.hello({ agent: "flora" });
  const { offer } = await (await owner.offer({ body: "race me", to: "agent:f*", slots: 2 })).json();

  const names = ["fable", "falcon", "fennel", "fiona", "flora"];
  const results = await Promise.all(names.map((name) => agent.claim({ agent: name, offer_id: offer.id })));
  const won = results.filter((r) => r.status === 200);
  const lost = results.filter((r) => r.status === 400);
  assert.equal(won.length, 2, "exactly the slot count wins");
  assert.equal(lost.length, 3);
  for (const r of lost) assert.match((await r.json()).error, /filled/);

  const list = (await (await owner.offers()).json()).offers;
  assert.equal(list[0].claimed, 2);
  assert.equal(list[0].state, "filled");
  const delivered = (await (await owner.messages({ direction: "inbound" })).json()).messages;
  assert.equal(delivered.length, 2, "one mailbox row per winning claim, no more");
});

test("a claim after the offer's expiry is refused, and the offer reads expired", async () => {
  const { agent, owner, advance } = await harness();
  await register(agent);
  const { offer } = await (await owner.offer({ body: "short lived", to: "all", slots: 1, ttl_seconds: 60 })).json();
  assert.ok(offer.expires_at > offer.ts);
  advance(61);
  const res = await agent.claim({ agent: "fable", offer_id: offer.id });
  assert.equal(res.status, 400);
  assert.match((await res.json()).error, /expired/);
  assert.deepEqual((await (await agent.offers({ agent: "fable" })).json()).offers, [], "expired offers leave the open list");
  const list = (await (await owner.offers()).json()).offers;
  assert.equal(list[0].state, "expired");
  assert.equal(list[0].claimed, 0);
});

test("a second claim by the same agent is refused and delivers nothing twice", async () => {
  const { agent, owner } = await harness();
  await register(agent);
  const { offer } = await (await owner.offer({ body: "once only", to: "all", slots: 3 })).json();
  assert.equal((await agent.claim({ agent: "fable", offer_id: offer.id })).status, 200);
  const again = await agent.claim({ agent: "fable", offer_id: offer.id });
  assert.equal(again.status, 400);
  assert.match((await again.json()).error, /already claimed/);
  assert.equal((await (await agent.inbox({ agent: "fable", unread_only: "false" })).json()).messages.length, 1);
  const list = (await (await owner.offers()).json()).offers;
  assert.equal(list[0].claimed, 1);
  assert.equal(list[0].state, "open", "slots remain for other agents");
  // And the claimer no longer sees it as open.
  assert.deepEqual((await (await agent.offers({ agent: "fable" })).json()).offers, []);
});

test("the poster cannot claim its own offer", async () => {
  const { agent, owner } = await harness();
  await register(agent);
  const posted = await (await agent.offer({ agent: "muse-1", body: "for the crew", to: "group:crew", slots: 2 })).json();
  const own = await agent.claim({ agent: "muse-1", offer_id: posted.offer.id });
  assert.equal(own.status, 400);
  assert.match((await own.json()).error, /not eligible/);
  assert.equal((await agent.claim({ agent: "falcon", offer_id: posted.offer.id })).status, 200);
  const list = (await (await owner.offers()).json()).offers;
  assert.deepEqual(list[0].claims.map((c) => c.agent), ["falcon"]);
});

test("offer and claim input is checked", async () => {
  const { agent, owner } = await harness();
  await register(agent);
  for (const body of [
    { body: "x", to: "all" }, // no slots
    { body: "x", to: "all", slots: 0 },
    { body: "x", to: "all", slots: 201 },
    { body: "x", to: "all", slots: 1.5 },
    { body: "", to: "all", slots: 1 },
    { body: "x", slots: 1 }, // no address
    { body: "x", to: "agent:z*", slots: 1, ttl_seconds: 0 },
    { body: "x", to: "all", slots: 1, kind: "shout" },
  ]) {
    const r = await owner.offer(body);
    assert.equal(r.status, 400, JSON.stringify(body));
    assert.ok((await r.json()).error);
  }
  const missing = await agent.claim({ agent: "fable", offer_id: 999 });
  assert.equal(missing.status, 400);
  assert.match((await missing.json()).error, /does not exist/);
  assert.equal((await agent.claim({ agent: "fable" })).status, 400);
});

test("a per-agent credential claims and posts only as itself, within its scopes", async () => {
  const { store, post, call, ft, agent, owner } = await harness();
  await register(agent);
  const { offer } = await (await owner.offer({ body: "cred task", to: "agent:f*", slots: 2 })).json();

  // A credential for fable with only the send scope: it may post an
  // offer but may not claim (claiming acts on its mailbox: inbox scope).
  const sendOnly = "s".repeat(43);
  await store.fleetCredentialPut({ token_hash: await noteTokenHash(sendOnly), agent: "fable", scopes: ["send"], created: "2026-11-24T00:00:00.000Z" });
  const denied = await post(`/fleet/${sendOnly}/claim`, { agent: "fable", offer_id: offer.id });
  assert.equal(denied.status, 404, "no inbox scope: the plain 404 every refusal gets");
  const posted = await post(`/fleet/${sendOnly}/offer`, { agent: "fable", body: "from cred", to: "all", slots: 1 });
  assert.equal(posted.status, 201);
  assert.equal((await posted.json()).offer.sender_addr, "agent:fable", "the credential fixes the sender to its own name");

  // A credential with both scopes claims as itself — and only itself.
  const both = "b".repeat(43);
  await store.fleetCredentialPut({ token_hash: await noteTokenHash(both), agent: "falcon", scopes: ["inbox", "send"], created: "2026-11-24T00:00:00.000Z" });
  const asOther = await post(`/fleet/${both}/claim`, { agent: "fable", offer_id: offer.id });
  assert.equal(asOther.status, 404, "a credential cannot claim for another agent");
  const mine = await post(`/fleet/${both}/claim`, { agent: "falcon", offer_id: offer.id });
  assert.equal(mine.status, 200);
  assert.equal((await mine.json()).message.agent, "falcon");
  const seen = await call(`/fleet/${both}/offers?agent=falcon`);
  assert.equal(seen.status, 200);
  const seenOther = await call(`/fleet/${both}/offers?agent=fable`);
  assert.equal(seenOther.status, 404);
  assert.ok(ft.length > 0);
});
