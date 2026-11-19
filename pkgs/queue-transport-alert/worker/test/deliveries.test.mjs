// Phone push, post-outage queue task and the public status view (code#6369).
import assert from "node:assert/strict";
import { test } from "node:test";
import { makeTelegramNotifier } from "../src/notify.js";
import { buildOutagePacket, makeBrokerEnqueuer } from "../src/queue.js";
import { runPass } from "../src/run.js";
import { publicStatus, renderStatusPage } from "../src/status.js";

const OK = { ok: true, detail: "" };
const FAIL = { ok: false, detail: "HTTP 530 (the tunnel could not reach the broker)" };
const t = (s) => Date.parse(s);

class World {
  constructor() {
    this.state = null;
    this.pushes = [];
    this.enqueued = [];
    this.failPush = false;
    this.failEnqueue = false;
    this.n = 9000;
    this.github = {
      find: async () => null,
      file: async () => ++this.n,
      comment: async () => {},
    };
    this.notify = async (text) => {
      if (this.failPush) throw new Error("telegram down");
      this.pushes.push(text);
    };
    this.enqueue = async (item) => {
      if (this.failEnqueue) throw new Error("broker refused");
      this.enqueued.push(item);
    };
  }
  async pass(probe, at) {
    const r = await runPass(this.state, probe, this.github, t(at), 2, "probe", {
      notify: this.notify,
      enqueue: this.enqueue,
      statusUrl: "https://status.example/",
    });
    this.state = r.state;
    return r;
  }
}

test("alert pushes once, with the incident link and status page", async () => {
  const w = new World();
  await w.pass(FAIL, "2026-11-19T04:00:00Z");
  assert.equal(w.pushes.length, 0);
  await w.pass(FAIL, "2026-11-19T04:10:00Z");
  assert.equal(w.pushes.length, 1);
  assert.match(w.pushes[0], /^Queue broker DOWN/);
  assert.match(w.pushes[0], /issues\/9001/);
  assert.match(w.pushes[0], /https:\/\/status\.example\//);
  await w.pass(FAIL, "2026-11-19T04:20:00Z");
  assert.equal(w.pushes.length, 1, "a continuing outage does not push again");
});

test("a failed push is retried next tick, not lost or doubled", async () => {
  const w = new World();
  w.failPush = true;
  await w.pass(FAIL, "2026-11-19T04:00:00Z");
  const r = await w.pass(FAIL, "2026-11-19T04:10:00Z");
  assert.match(r.errors.join(" "), /telegram down/);
  assert.deepEqual(w.state.pending.map((p) => p.type), ["notify"]);
  w.failPush = false;
  await w.pass(FAIL, "2026-11-19T04:20:00Z");
  assert.equal(w.pushes.length, 1);
  assert.deepEqual(w.state.pending, []);
});

test("recovery pushes once and files the queue task while the broker answers", async () => {
  const w = new World();
  await w.pass(FAIL, "2026-11-19T04:00:00Z");
  await w.pass(FAIL, "2026-11-19T04:10:00Z");
  const r = await w.pass(OK, "2026-11-19T05:00:00Z");
  assert.equal(r.event, "recovered");
  assert.equal(w.pushes.length, 2);
  assert.match(w.pushes[1], /^Queue broker back UP/);
  assert.deepEqual(w.enqueued, [{ type: "enqueue", incident: 9001, start: "2026-11-19T04:00:00Z", end: "2026-11-19T05:00:00Z" }]);
  assert.deepEqual(w.state.last_outage, { start: "2026-11-19T04:00:00Z", end: "2026-11-19T05:00:00Z", incident: 9001 });
  assert.equal(r.incident, 9001);
});

test("a refused enqueue waits and is retried on a later healthy tick", async () => {
  const w = new World();
  await w.pass(FAIL, "2026-11-19T04:00:00Z");
  await w.pass(FAIL, "2026-11-19T04:10:00Z");
  w.failEnqueue = true;
  await w.pass(OK, "2026-11-19T05:00:00Z");
  assert.deepEqual(w.state.pending.map((p) => p.type), ["enqueue"]);
  w.failEnqueue = false;
  await w.pass(FAIL, "2026-11-19T05:10:00Z"); // broker down again: hold it
  assert.equal(w.enqueued.length, 0);
  await w.pass(OK, "2026-11-19T05:20:00Z");
  assert.equal(w.enqueued.length, 1);
  assert.deepEqual(w.state.pending, []);
  assert.ok(w.state.last_outage, "a later blip keeps the last outage");
});

test("no clients configured: nothing is delivered and nothing piles up", async () => {
  let state = null;
  for (const [p, at] of [
    [FAIL, "2026-11-19T04:00:00Z"],
    [FAIL, "2026-11-19T04:10:00Z"],
    [OK, "2026-11-19T04:20:00Z"],
  ]) {
    ({ state } = await runPass(state, p, null, t(at), 2, "x"));
  }
  assert.deepEqual(state.pending, []);
});

test("outage packet passes the repo-task/v1 contract the broker checks", () => {
  const p = buildOutagePacket(
    { incident: 9001, start: "2026-11-19T04:00:00Z", end: "2026-11-19T05:00:00Z" },
    "developing-today/code",
    t("2026-11-19T05:00:00Z"),
  );
  assert.equal(p.dedupe_key, "repo-health:v1:triage:developing-today/code:issue:9001");
  assert.ok([10, 15, 20, 25, 30, 40, 50, 70, 90].includes(p.priority));
  const y = p.payload;
  assert.equal(y.schema, "repo-task/v1");
  assert.equal(y.kind, "triage");
  assert.equal(y.issue.number, 9001);
  assert.equal(y.timebox_minutes, 20);
  assert.equal(y.max_attempts, 1000);
  for (const f of ["evidence", "acceptance", "tests", "context_refs"]) assert.ok(y[f].length > 0, f);
  assert.ok(y.scope.exclude.length > 0);
  assert.equal(y.risk.level, "low");
  assert.ok(y.dedupe.checked_at && y.dedupe.basis);
  assert.match(p.body, /Do not close the incident/);
});

test("enqueuer sends the token headers and treats a dedupe hit as done", async () => {
  const seen = [];
  const mk = (resp) =>
    makeBrokerEnqueuer(
      { brokerUrl: "https://b.example/", agentToken: "at", accessId: "ci", accessSecret: "cs", queue: "q", repo: "developing-today/code" },
      async (url, init) => {
        seen.push({ url, init });
        return resp();
      },
    );
  const item = { incident: 1, start: "2026-11-19T04:00:00Z", end: "2026-11-19T05:00:00Z" };
  await mk(() => new Response(JSON.stringify({ ok: true, row: { id: 1 } })))(item);
  assert.equal(seen[0].url, "https://b.example/enqueue");
  assert.equal(seen[0].init.headers["X-Agent-Token"], "at");
  assert.equal(seen[0].init.headers["CF-Access-Client-Id"], "ci");
  assert.equal(JSON.parse(seen[0].init.body).queue, "q");
  await mk(() => new Response(JSON.stringify({ ok: false, error: "duplicate dedupe_key" })))(item);
  await assert.rejects(mk(() => new Response("bad gateway", { status: 502 }))(item), /HTTP 502/);
});

test("Telegram notifier posts to the chat and never echoes the token", async () => {
  let call;
  const n = makeTelegramNotifier("SECRET", "42", async (url, init) => {
    call = { url, init };
    return new Response(JSON.stringify({ ok: true }));
  });
  await n("hello");
  assert.equal(JSON.parse(call.init.body).chat_id, "42");
  const bad = makeTelegramNotifier("SECRET", "42", async () => new Response('{"ok":false,"description":"chat not found"}', { status: 400 }));
  await assert.rejects(bad("x"), (err) => !String(err.message).includes("SECRET") && /HTTP 400/.test(err.message));
});

test("status view: up, degraded, down, stale, unknown", () => {
  const env = { REPO: "developing-today/code", BROKER_URL: "https://b", THRESHOLD: "2" };
  const now = t("2026-11-19T05:00:00Z");
  assert.equal(publicStatus(null, env, now).status, "unknown");
  const up = publicStatus({ last_probe: "2026-11-19T04:55:00Z", last_ok: "2026-11-19T04:55:00Z", consecutive_failures: 0 }, env, now);
  assert.equal(up.status, "up");
  assert.equal(up.up, true);
  assert.equal(publicStatus({ last_probe: "2026-11-19T04:55:00Z", consecutive_failures: 1 }, env, now).status, "degraded");
  const down = publicStatus(
    { last_probe: "2026-11-19T04:55:00Z", consecutive_failures: 3, alerted: true, incident: 7, window_start: "2026-11-19T04:30:00Z" },
    env,
    now,
  );
  assert.equal(down.status, "down");
  assert.equal(down.up, false);
  assert.equal(down.incident.url, "https://github.com/developing-today/code/issues/7");
  assert.equal(publicStatus({ last_probe: "2026-11-19T04:00:00Z" }, env, now).status, "stale");
  const withOutage = publicStatus(
    { last_probe: "2026-11-19T04:55:00Z", last_outage: { start: "2026-11-19T01:00:00Z", end: "2026-11-19T02:15:00Z", incident: 7 } },
    env,
    now,
  );
  assert.equal(withOutage.last_outage.duration, "1h15m");
  const html = renderStatusPage(withOutage);
  assert.match(html, /<title>Queue broker status<\/title>/);
  assert.match(html, /1h15m/);
  const page = renderStatusPage({ ...up, last_failure: "<script>alert(1)</script>" });
  assert.match(page, /&lt;script&gt;alert\(1\)/);
  assert.equal(page.match(/<script>/g).length, 1, "only the page's own script");
  assert.match(page, /rel="manifest"/);
});
