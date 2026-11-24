// The instruction work-item lifecycle (src/fleet.js, src/notes.js,
// src/pipeline.js; design §4.6, #14372): a received instruction is a
// work item whose state the RECEIVING agent records — acted in-session,
// queued as a named fleet-queue task, then done — forward only, on its
// own copy only. Routed end to end through route(), with the real
// fleetOp SQL on node:sqlite and MemoryStore for the connector and
// notes surfaces — the same harness shape as pipeline.test.mjs and
// credentials.test.mjs. All token values are fixtures or Worker-minted
// inside the test; nothing here is a real credential.
//
// Oracles (named for the acceptance criteria):
// - an instruction's lifecycle is settable only by its receiving agent,
//   and only forward through the states;
// - the owner pipeline shows an instruction at `queued` with its queue
//   task id, and at `done` after the agent reports done;
// - group recipients carry independent lifecycles on the same batch;
// - connector instructions join at received/acted via read state.
import assert from "node:assert/strict";
import { test } from "node:test";
import { DatabaseSync } from "node:sqlite";
import { fleetCall, fleetOp, fleetSchema, fleetToken } from "../src/fleet.js";
import { MemoryStore } from "../src/messages.js";
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
  /** Mints and exchanges a bootstrap token; returns the agent's fleet credential. */
  const enroll = async (agent) => {
    const mint = await post(`/api/${TOKEN}/enroll-tokens`, { agent, kind: "muse", groups: ["muse"] });
    assert.equal(mint.status, 201);
    const { bootstrap_token } = await mint.json();
    const ex = await post(`/enroll/exchange`, { bootstrap_token, agent });
    assert.equal(ex.status, 201);
    return (await ex.json()).fleet_token;
  };
  return {
    store,
    ft,
    enroll,
    pipeline: (qs = {}) => call(`/api/${TOKEN}/pipeline?${new URLSearchParams(qs)}`),
    register: (identity, token, extra = {}) => post("/notes/register", { identity, token, ...extra }, bearer(TOKEN)),
    noteSend: (token, to, note) => post("/notes/send", { to, note }, bearer(token)),
    noteWork: (token, body) => post("/notes/work", body, bearer(token)),
    fleetHello: (body) => post(`/fleet/${ft}/hello`, body),
    fleetInbox: (qs) => call(`/fleet/${ft}/inbox?${new URLSearchParams(qs)}`),
    fleetWork: (body) => post(`/fleet/${ft}/work`, body),
    credWork: (cred, body) => post(`/fleet/${cred}/work`, body),
    ownerFleetSend: (body) => post(`/api/${TOKEN}/fleet/send`, body),
    ownerSend: (body) => post(`/api/${TOKEN}/messages`, body),
  };
}

async function pipelineRow(h, body) {
  const res = await h.pipeline({ limit: "200" });
  assert.equal(res.status, 200);
  const { messages } = await res.json();
  return messages.find((m) => m.body === body);
}

test("work: a fleet instruction moves received -> queued (with its task id) -> done, and the pipeline shows each step", async () => {
  const h = await harness();
  await h.fleetHello({ agent: "felix", groups: ["muse"] });
  const sent = await (await h.ownerFleetSend({ body: "do the thing", to: "agent:felix", kind: "command" })).json();
  const id = sent.messages[0].id;

  // Unmarked, the pipeline reads the row as received, with no task.
  let row = await pipelineRow(h, "do the thing");
  assert.equal(row.surface, "fleet");
  assert.equal(row.work_state, "received");
  assert.equal(row.work_task_id, null);

  // The receiving agent queues it, naming the queue task it became.
  const marked = await h.fleetWork({ agent: "felix", id, state: "queued", task_id: 23500 });
  assert.equal(marked.status, 200);
  const markedBody = await marked.json();
  assert.equal(markedBody.message.work_state, "queued");
  assert.equal(markedBody.message.work_task_id, 23500);

  row = await pipelineRow(h, "do the thing");
  assert.equal(row.work_state, "queued", "the owner pipeline shows the instruction at queued");
  assert.equal(row.work_task_id, 23500, "... with its queue task id");

  // The work_state filter finds exactly the stuck/queued views.
  const queued = await (await h.pipeline({ work_state: "queued" })).json();
  assert.deepEqual(queued.messages.map((m) => m.body), ["do the thing"]);
  const stuck = await (await h.pipeline({ work_state: "received", surface: "fleet" })).json();
  assert.deepEqual(stuck.messages, []);

  // The agent's own inbox row carries the state it recorded.
  const inbox = await (await h.fleetInbox({ agent: "felix", unread_only: "false" })).json();
  assert.equal(inbox.messages[0].work_state, "queued");
  assert.equal(inbox.messages[0].work_task_id, 23500);

  // Done, reported by the agent; the recorded task id survives.
  const done = await h.fleetWork({ agent: "felix", id, state: "done" });
  assert.equal(done.status, 200);
  row = await pipelineRow(h, "do the thing");
  assert.equal(row.work_state, "done", "the pipeline shows done after the agent reports done");
  assert.equal(row.work_task_id, 23500);
});

test("work: transitions move only forward (acted | queued -> done), and queued names its task", async () => {
  const h = await harness();
  await h.fleetHello({ agent: "felix", groups: ["muse"] });
  const sent = await (await h.ownerFleetSend({ body: "forward only", to: "agent:felix", kind: "command" })).json();
  const id = sent.messages[0].id;

  // received -> done skips the middle: refused.
  let res = await h.fleetWork({ agent: "felix", id, state: "done" });
  assert.equal(res.status, 400);
  assert.match((await res.json()).error, /only forward/);

  // queued without the task it became: refused at validation.
  res = await h.fleetWork({ agent: "felix", id, state: "queued" });
  assert.equal(res.status, 400);
  assert.match((await res.json()).error, /task_id/);

  // acted, then sideways to queued: refused. Acted again: a no-op.
  assert.equal((await h.fleetWork({ agent: "felix", id, state: "acted" })).status, 200);
  res = await h.fleetWork({ agent: "felix", id, state: "queued", task_id: 1 });
  assert.equal(res.status, 400);
  assert.match((await res.json()).error, /only forward/);
  assert.equal((await h.fleetWork({ agent: "felix", id, state: "acted" })).status, 200, "re-marking the current state is a no-op");

  // done, then anything else: refused.
  assert.equal((await h.fleetWork({ agent: "felix", id, state: "done" })).status, 200);
  res = await h.fleetWork({ agent: "felix", id, state: "acted" });
  assert.equal(res.status, 400);

  // A state outside the lifecycle: refused at validation.
  res = await h.fleetWork({ agent: "felix", id, state: "received" });
  assert.equal(res.status, 400);
});

test("work: group recipients carry independent lifecycles on the same batch", async () => {
  const h = await harness();
  await h.fleetHello({ agent: "felix", groups: ["muse"] });
  await h.fleetHello({ agent: "archer", groups: ["muse"] });
  const sent = await (await h.ownerFleetSend({ body: "group order", to: "group:muse", kind: "command" })).json();
  assert.equal(sent.messages.length, 2);
  const batch = sent.messages[0].batch;
  assert.ok(sent.messages.every((m) => m.batch === batch), "one send, one batch");
  const felixRow = sent.messages.find((m) => m.agent === "felix");
  const archerRow = sent.messages.find((m) => m.agent === "archer");

  assert.equal((await h.fleetWork({ agent: "felix", id: felixRow.id, state: "queued", task_id: 42 })).status, 200);

  const res = await h.pipeline({ surface: "fleet", batch });
  const { messages } = await res.json();
  const byAgent = new Map(messages.map((m) => [m.agent, m]));
  assert.equal(byAgent.get("felix").work_state, "queued");
  assert.equal(byAgent.get("felix").work_task_id, 42);
  assert.equal(byAgent.get("archer").work_state, "received", "the other recipient's copy is unmoved");

  // Archer's copy moves on its own track.
  assert.equal((await h.fleetWork({ agent: "archer", id: archerRow.id, state: "acted" })).status, 200);
  const after = await (await h.pipeline({ surface: "fleet", batch })).json();
  const afterByAgent = new Map(after.messages.map((m) => [m.agent, m]));
  assert.equal(afterByAgent.get("archer").work_state, "acted");
  assert.equal(afterByAgent.get("felix").work_state, "queued");
});

test("work: a credential marks only its own rows — cross-agent marking is a plain 404", async () => {
  const h = await harness();
  const felixCred = await h.enroll("felix");
  const archerCred = await h.enroll("archer");
  const sent = await (await h.ownerFleetSend({ body: "for felix only", to: "agent:felix", kind: "command" })).json();
  const id = sent.messages[0].id;

  // Archer's credential naming felix: refused at the name check.
  let res = await h.credWork(archerCred, { agent: "felix", id, state: "acted" });
  assert.equal(res.status, 404);
  // Archer's credential naming itself but felix's row id: the row is
  // not in archer's mailbox, and the refusal is the same plain 404.
  res = await h.credWork(archerCred, { agent: "archer", id, state: "acted" });
  assert.equal(res.status, 404);
  // Felix's row is unmoved by either attempt.
  let row = await pipelineRow(h, "for felix only");
  assert.equal(row.work_state, "received");

  // Felix's own credential marks it — no owner token involved.
  res = await h.credWork(felixCred, { agent: "felix", id, state: "acted" });
  assert.equal(res.status, 200, await res.text().catch(() => ""));
  row = await pipelineRow(h, "for felix only");
  assert.equal(row.work_state, "acted");

  // On the shared-token path a missing row is the ordinary 400 refusal.
  res = await h.fleetWork({ agent: "felix", id: 999999, state: "acted" });
  assert.equal(res.status, 400);
});

test("work: notes instructions reuse the lifecycle on the notes surface", async () => {
  const h = await harness();
  await h.register("user/drew", DREW);
  await h.register("muse/felix", FELIX);
  await h.register("muse/archer", ARCHER);
  const delivered = await (await h.noteSend(DREW, "muse/felix", "note instruction")).json();
  const id = delivered.notes[0].id;

  let row = await pipelineRow(h, "note instruction");
  assert.equal(row.surface, "notes");
  assert.equal(row.work_state, "received");
  assert.equal(row.work_task_id, null);

  // Another identity cannot move felix's note, and learns nothing.
  const cross = await h.noteWork(ARCHER, { id, state: "acted" });
  assert.equal(cross.status, 400);
  assert.match((await cross.json()).error, /not in muse\/archer's inbox/);
  row = await pipelineRow(h, "note instruction");
  assert.equal(row.work_state, "received", "the cross-identity attempt changed nothing");

  // The receiving identity queues it with its task id, then finishes.
  const marked = await h.noteWork(FELIX, { id, state: "queued", task_id: 777 });
  assert.equal(marked.status, 200);
  assert.deepEqual(await marked.json(), { id, work_state: "queued", work_task_id: 777 });
  row = await pipelineRow(h, "note instruction");
  assert.equal(row.work_state, "queued");
  assert.equal(row.work_task_id, 777);

  assert.equal((await h.noteWork(FELIX, { id, state: "done" })).status, 200);
  const backward = await h.noteWork(FELIX, { id, state: "acted" });
  assert.equal(backward.status, 400, "done -> acted is a backward step");
  row = await pipelineRow(h, "note instruction");
  assert.equal(row.work_state, "done");
});

test("work: connector instructions join at received/acted through read state alone", async () => {
  const h = await harness();
  await h.ownerSend({ body: "connector instruction", sender: "drew", to: "claude", kind: "instruction" });
  await h.ownerSend({ body: "connector message", sender: "drew", to: "claude", kind: "message" });

  let row = await pipelineRow(h, "connector instruction");
  assert.equal(row.surface, "connector");
  assert.equal(row.work_state, "received", "unread connector instruction reads as received");
  assert.equal(row.work_task_id, null);
  row = await pipelineRow(h, "connector message");
  assert.equal(row.work_state, null, "a plain connector message is not a work item");

  const { messages } = await (await h.pipeline({ surface: "connector" })).json();
  const instruction = messages.find((m) => m.body === "connector instruction");
  await h.store.ack({ ids: [instruction.id], channel: "claude" });

  row = await pipelineRow(h, "connector instruction");
  assert.equal(row.work_state, "acted", "read state stands in for acted on the connector surface");
});
