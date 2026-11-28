// The owner's unified pipeline (src/pipeline.js): one feed over
// connector messages, fleet mailboxes and notes, the owner's notes
// read, and the unified agents list. Routed end to end through
// route(), with the real fleetOp SQL on node:sqlite and MemoryStore
// for the connector and notes surfaces — the same harness shape as
// fleet.test.mjs and notes.test.mjs.
//
// Oracles: the owner view is never sender-stripped (a fleet command, a
// connector instruction and a note show their real senders side by
// side), the cross-surface filters return exactly the matching rows,
// and the recipient-side projections (fleet commandForAgent via the
// agent inbox, messages.js forAgent) are unchanged.
import assert from "node:assert/strict";
import { test } from "node:test";
import { DatabaseSync } from "node:sqlite";
import { fleetCall, fleetOp, fleetSchema, fleetToken } from "../src/fleet.js";
import { MemoryStore, forAgent } from "../src/messages.js";
import { route } from "../src/routes.js";

const TOKEN = "o".repeat(20) + "0123456789abcdefXYZ_-";
const CHATGPT = "c".repeat(40);
const DREW = "d".repeat(40);
const FELIX = "f".repeat(40);
const ARCHER = "a".repeat(40);
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
  const stub = {
    fleet: (op, args) => fleetOp(sql, op, args, `2026-11-24T00:00:${String(clock++).padStart(2, "0")}Z`),
  };
  const store = new MemoryStore(() => "2026-11-24T00:00:00.000Z");
  store.fleet = (op, args) => fleetCall(stub, op, args);
  const call = (path, init) => route(new Request(BASE + path, init), env, () => store);
  const post = (path, body, headers = {}) =>
    call(path, { method: "POST", headers: { "content-type": "application/json", ...headers }, body: JSON.stringify(body) });
  const bearer = (token) => ({ authorization: `Bearer ${token}` });
  const ft = await fleetToken(env);
  return {
    store,
    call,
    ft,
    pipeline: (qs = {}) => call(`/api/${TOKEN}/pipeline?${new URLSearchParams(qs)}`),
    notesOwner: (qs = {}) => call(`/api/${TOKEN}/notes?${new URLSearchParams(qs)}`),
    agents: () => call(`/api/${TOKEN}/agents`),
    register: (identity, token, extra = {}) =>
      post("/notes/register", { identity, token, ...extra }, bearer(TOKEN)),
    noteSend: (token, to, note) => post("/notes/send", { to, note }, bearer(token)),
    noteAck: (token, body) => post("/notes/ack", body, bearer(token)),
    fleetHello: (body) => post(`/fleet/${ft}/hello`, body),
    fleetInbox: (qs) => call(`/fleet/${ft}/inbox?${new URLSearchParams(qs)}`),
    fleetSend: (body) => post(`/fleet/${ft}/send`, body),
    ownerFleetSend: (body) => post(`/api/${TOKEN}/fleet/send`, body),
    ownerSend: (body) => post(`/api/${TOKEN}/messages`, body),
  };
}

/**
 * One of everything, across all three surfaces:
 * - a fleet command from Drew to felix (batch B)
 * - a fleet message from felix back to central (outbound post)
 * - a connector instruction felix relayed to Claude (from_addr agent:felix)
 * - a connector instruction Drew sent to Claude himself
 * - a plain connector message to ChatGPT
 * - a note from Drew to muse/felix, and one from muse/felix to muse/archer
 */
async function seeded() {
  const h = await harness();
  await h.register("user/drew", DREW);
  await h.register("muse/felix", FELIX);
  await h.register("muse/archer", ARCHER, { can_send: false });
  await h.fleetHello({ agent: "felix", groups: ["muse"] });
  await h.fleetHello({ agent: "archer", groups: ["muse"] });

  const cmd = await (await h.ownerFleetSend({ body: "fleet command body", to: "agent:felix", kind: "command", sender: "drew" })).json();
  assert.equal(cmd.messages.length, 1);
  const batch = cmd.messages[0].batch;
  assert.ok(batch);

  const relayed = await h.fleetSend({ agent: "felix", body: "agent instruction body", to: "claude", kind: "instruction" });
  assert.equal(relayed.status, 201, await relayed.text().catch(() => ""));
  await h.fleetSend({ agent: "felix", body: "report to central", reply_to: cmd.messages[0].id });

  await h.ownerSend({ body: "owner instruction body", sender: "drew", to: "claude", kind: "instruction" });
  await h.ownerSend({ body: "hello chatgpt", sender: "drew", to: "chatgpt", kind: "message" });

  await h.noteSend(DREW, "muse/felix", "note for felix");
  await h.noteSend(FELIX, "muse/archer", "felix note to archer");
  return { ...h, batch };
}

test("pipeline: one query shows a fleet command, a connector instruction and a note, real senders on all three", async () => {
  const { pipeline } = await seeded();
  const res = await pipeline({ limit: "200" });
  assert.equal(res.status, 200);
  const { messages } = await res.json();
  const byBody = new Map(messages.map((m) => [m.body, m]));

  const cmd = byBody.get("fleet command body");
  assert.equal(cmd.surface, "fleet");
  assert.equal(cmd.kind, "command");
  assert.equal(cmd.sender_addr, "user:drew", "owner view: the command's real sender is shown");
  assert.equal(cmd.sender_label, "drew");
  assert.equal(cmd.receiver, "agent:felix");
  assert.equal(cmd.direction, "inbound");
  assert.ok(cmd.batch);

  const relayed = byBody.get("agent instruction body");
  assert.equal(relayed.surface, "connector");
  assert.equal(relayed.kind, "instruction");
  assert.equal(relayed.sender_addr, "agent:felix", "owner view: the instruction's real sender is shown");
  assert.equal(relayed.receiver, "channel:claude");

  const own = byBody.get("owner instruction body");
  assert.equal(own.kind, "instruction");
  assert.equal(own.sender_addr, "user:drew");
  assert.equal(own.sender_label, "drew");

  const note = byBody.get("note for felix");
  assert.equal(note.surface, "notes");
  assert.equal(note.kind, "note");
  assert.equal(note.sender_addr, "user/drew");
  assert.equal(note.receiver, "muse/felix");

  const report = byBody.get("report to central");
  assert.equal(report.surface, "fleet");
  assert.equal(report.direction, "outbound");
  assert.equal(report.sender_addr, "agent:felix");
  assert.equal(report.receiver, "user:drew");
  assert.equal(report.reply_to, cmd.id, "the reply threads to the fleet row it answers (per-surface id)");

  for (const m of messages) assert.ok(m.sender_addr, `no pipeline row is sender-stripped: ${JSON.stringify(m)}`);
  const stamps = messages.map((m) => m.ts);
  assert.deepEqual(stamps, [...stamps].sort(), "default order is oldest first across surfaces");
});

test("pipeline filters compose across surfaces: kind+sender and receiver return exactly the matching rows", async () => {
  const { pipeline, batch } = await seeded();

  // Oracle 1: kind=instruction & sender=agent:felix — only the relayed
  // connector instruction. The fleet command is kind "command" (the
  // fleet spelling), Drew's instruction has another sender.
  let rows = (await (await pipeline({ kind: "instruction", sender: "agent:felix" })).json()).messages;
  assert.deepEqual(rows.map((m) => [m.surface, m.body]), [["connector", "agent instruction body"]]);

  // Oracle 2: receiver=felix — the fleet command (agent:felix) and the
  // note (muse/felix), nothing else, on two surfaces.
  rows = (await (await pipeline({ receiver: "felix" })).json()).messages;
  assert.deepEqual(
    rows.map((m) => [m.surface, m.body]).sort(),
    [
      ["fleet", "fleet command body"],
      ["notes", "note for felix"],
    ],
  );
  // The full identity form narrows the same filter to the note alone.
  rows = (await (await pipeline({ receiver: "muse/felix" })).json()).messages;
  assert.deepEqual(rows.map((m) => m.body), ["note for felix"]);
  rows = (await (await pipeline({ receiver: "agent:felix" })).json()).messages;
  assert.deepEqual(rows.map((m) => m.body), ["fleet command body"]);

  // Surface, kind and batch filters each stay inside their surface.
  rows = (await (await pipeline({ surface: "notes" })).json()).messages;
  assert.ok(rows.length === 2 && rows.every((m) => m.surface === "notes"));
  rows = (await (await pipeline({ kind: "command" })).json()).messages;
  assert.deepEqual(rows.map((m) => m.body), ["fleet command body"]);
  rows = (await (await pipeline({ batch })).json()).messages;
  assert.deepEqual(rows.map((m) => m.body).sort(), ["fleet command body", "report to central"]);

  // Sender in bare-name and identity forms names the same note sender.
  rows = (await (await pipeline({ sender: "muse/felix" })).json()).messages;
  assert.ok(rows.some((m) => m.body === "felix note to archer"));
  assert.ok(rows.every((m) => m.sender_addr === "muse/felix" || m.sender_addr === "agent:felix"));
});

test("pipeline unread_only and the owner notes read share the ack state", async () => {
  const h = await seeded();
  // Felix acks Drew's note through the identity API; the owner reads
  // must agree it is read.
  const inbox = await (await h.call("/notes/inbox", { headers: { authorization: `Bearer ${FELIX}` } })).json();
  assert.equal(inbox.notes.length, 1);
  await h.noteAck(FELIX, { ids: [inbox.notes[0].id] });

  const unread = (await (await h.pipeline({ unread_only: "true", limit: "200" })).json()).messages;
  assert.ok(!unread.some((m) => m.body === "note for felix"), "acked note is not unread in the pipeline");
  assert.ok(unread.some((m) => m.body === "felix note to archer"), "unacked note is unread");
  assert.ok(unread.every((m) => m.direction === "inbound" && m.read_at === null));

  const all = (await (await h.notesOwner({})).json()).notes;
  assert.equal(all.length, 2);
  const acked = all.find((n) => n.body === "note for felix");
  assert.equal(acked.sender, "user/drew");
  assert.equal(acked.recipient, "muse/felix");
  assert.ok(acked.ack_at, "owner read shows the ack the identity API recorded");

  const onlyUnread = (await (await h.notesOwner({ unread_only: "true" })).json()).notes;
  assert.deepEqual(onlyUnread.map((n) => n.body), ["felix note to archer"]);
  const toArcher = (await (await h.notesOwner({ receiver: "muse/archer" })).json()).notes;
  assert.deepEqual(toArcher.map((n) => [n.sender, n.body]), [["muse/felix", "felix note to archer"]]);
  const fromDrew = (await (await h.notesOwner({ sender: "user/drew" })).json()).notes;
  assert.deepEqual(fromDrew.map((n) => n.body), ["note for felix"]);
  const none = (await (await h.notesOwner({ kind: "instruction" })).json()).notes;
  assert.deepEqual(none, [], "notes are kind 'note' until S4; an instruction filter matches none");
});

test("agents list: a fleet agent and a notes-only identity side by side, correctly surfaced", async () => {
  const { agents } = await seeded();
  const res = await agents();
  assert.equal(res.status, 200);
  const rows = (await res.json()).agents;
  const byKey = new Map(rows.map((a) => [`${a.surface}:${a.name}`, a]));

  const fleetFelix = byKey.get("fleet:felix");
  assert.deepEqual(fleetFelix.groups, ["muse"]);
  assert.ok(fleetFelix.first_seen && fleetFelix.last_seen, "fleet rows carry presence");
  assert.equal(fleetFelix.presence, "last_seen");
  assert.equal(fleetFelix.unread_commands, 0, "felix answered the command in the seed, so it is read");
  assert.ok(byKey.get("fleet:archer"), "the second fleet agent is listed");

  const noteFelix = byKey.get("notes:muse/felix");
  assert.deepEqual(noteFelix.scopes, { can_send: true, can_inbox: true });
  assert.equal(noteFelix.presence, "none");
  assert.equal(noteFelix.last_seen, null, "notes identities have no presence; none is invented");
  assert.equal(noteFelix.signal_counter, 1, "one note delivered to muse/felix in the seed");

  const noteArcher = byKey.get("notes:muse/archer");
  assert.deepEqual(noteArcher.scopes, { can_send: false, can_inbox: true }, "registration scopes are shown as registered");

  const drew = byKey.get("notes:user/drew");
  assert.ok(drew, "a notes-only identity (no fleet mailbox) is in the same list");
  assert.equal(drew.surface, "notes");
});

test("recipient projections are unchanged: no agent-facing response gains a sender", async () => {
  const h = await seeded();
  // The fleet command reaches felix's inbox with no sender, exactly as
  // before — while the pipeline test above shows the owner Drew's name.
  const inbox = await (await h.fleetInbox({ agent: "archer", unread_only: "false" })).json();
  assert.equal(inbox.messages.length, 0);
  const all = (await (await h.pipeline({ surface: "fleet", receiver: "felix" })).json()).messages;
  assert.ok(all.length >= 1);
  // A fresh command to archer, read through the agent route:
  await h.ownerFleetSend({ body: "for archer", to: "agent:archer", kind: "command", sender: "drew" });
  const box = await (await h.fleetInbox({ agent: "archer" })).json();
  assert.equal(box.messages.length, 1);
  assert.equal(box.messages[0].sender, null, "commandForAgent still strips the sender for the recipient");
  assert.equal(box.messages[0].from_addr, null);
  // And the connector projection still strips an instruction's sender.
  const stored = h.store.rows.find((r) => r.body === "agent instruction body");
  const seen = forAgent({ ...stored });
  assert.equal(seen.sender, undefined);
  assert.equal(seen.from_addr, null);
  assert.equal(stored.from_addr, "agent:felix", "the store row itself keeps the real sender");
});

test("pipeline routes are owner-only and validate their filters", async () => {
  const h = await seeded();
  for (const path of [`/api/nope/pipeline`, `/api/nope/notes`, `/api/nope/agents`, `/api/${h.ft}/pipeline`, `/api/${h.ft}/agents`]) {
    assert.equal((await h.call(path)).status, 404, path);
  }
  assert.equal((await h.pipeline({ surface: "bogus" })).status, 400);
  assert.equal((await h.pipeline({ kind: "bogus" })).status, 400);
  assert.equal((await h.pipeline({ limit: "0" })).status, 400);
  assert.equal((await h.pipeline({ order: "sideways" })).status, 400);
  const post = await h.call(`/api/${TOKEN}/pipeline`, { method: "POST", headers: { "content-type": "application/json" }, body: "{}" });
  assert.equal(post.status, 405);
});
