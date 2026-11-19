// Contract tests for the Worker's alert pass (code#6369). They mirror
// scripts/fleet/queue-transport-alert/test_check_transport.py so the
// Worker and the local probe keep one behaviour.
import assert from "node:assert/strict";
import { test } from "node:test";
import { classifyHealthz, INCIDENT_TITLE_PREFIX } from "../src/logic.js";
import { makeGitHub } from "../src/github.js";
import { runPass } from "../src/run.js";

class FakeGitHub {
  constructor(existing = null) {
    this.existing = existing;
    this.failOn = new Set();
    this.calls = [];
    this.next = 9000;
  }
  maybe(op) {
    if (this.failOn.has(op)) throw new Error(`${op} failed`);
  }
  async find() {
    this.calls.push(["find"]);
    this.maybe("find");
    return this.existing;
  }
  async file(title) {
    this.calls.push(["file", title]);
    this.maybe("file");
    return ++this.next;
  }
  async comment(n, body) {
    this.calls.push(["comment", n, body]);
    this.maybe("comment");
  }
  async close(n) {
    this.calls.push(["close", n]);
    this.maybe("close");
  }
  kinds() {
    return this.calls.map((c) => c[0]);
  }
}

const OK = { ok: true, detail: "" };
const FAIL = { ok: false, detail: "HTTP 530 (the tunnel could not reach the broker)" };
const t = (s) => Date.parse(s);

class Harness {
  constructor(gh = new FakeGitHub(), threshold = 2) {
    this.gh = gh;
    this.threshold = threshold;
    this.state = null;
  }
  async pass(probe, at) {
    const r = await runPass(this.state, probe, this.gh, t(at), this.threshold, "test probe");
    this.state = r.state;
    return r;
  }
  async alerted() {
    await this.pass(FAIL, "2026-11-18T23:00:00Z");
    await this.pass(FAIL, "2026-11-18T23:10:00Z");
  }
}

test("healthy pass is quiet", async () => {
  const h = new Harness();
  const r = await h.pass(OK, "2026-11-18T23:00:00Z");
  assert.equal(r.event, "ok");
  assert.equal(r.message, "");
  assert.deepEqual(h.gh.calls, []);
});

test("single failure does not alert", async () => {
  const h = new Harness();
  const r = await h.pass(FAIL, "2026-11-18T23:00:00Z");
  assert.equal(r.event, "failing");
  assert.deepEqual(h.gh.calls, []);
});

test("second failure files exactly one incident", async () => {
  const h = new Harness();
  await h.pass(FAIL, "2026-11-18T23:00:00Z");
  const r = await h.pass(FAIL, "2026-11-18T23:10:00Z");
  assert.equal(r.event, "alert");
  assert.deepEqual(h.gh.kinds(), ["find", "file"]);
  assert.equal(r.incident, 9001);
  assert.match(r.message, /unreachable for 10m/);
  assert.match(r.message, /#9001/);
  assert.ok(h.gh.calls[1][1].startsWith(INCIDENT_TITLE_PREFIX));
  assert.match(h.gh.calls[1][1], /2026-11-18T23:00:00Z/);
});

test("continuing outage stays quiet after the alert", async () => {
  const h = new Harness();
  let r;
  for (const m of ["00", "10", "20", "30", "40"]) r = await h.pass(FAIL, `2026-11-18T23:${m}:00Z`);
  assert.equal(r.event, "down");
  assert.equal(r.message, "");
  assert.deepEqual(h.gh.kinds(), ["find", "file"]);
  assert.equal(r.state.consecutive_failures, 5);
});

test("open incident is reused, not duplicated", async () => {
  const h = new Harness(new FakeGitHub(5708));
  await h.alerted();
  assert.deepEqual(h.gh.kinds(), ["find", "comment"]);
  assert.equal(h.gh.calls[1][1], 5708);
  assert.equal(h.state.incident, 5708);
  assert.equal(h.state.incident_filed_by_probe, false);
});

test("threshold is configurable", async () => {
  const h = new Harness(new FakeGitHub(), 1);
  const r = await h.pass(FAIL, "2026-11-18T23:00:00Z");
  assert.equal(r.event, "alert");
});

test("failed filing is retried on the next tick", async () => {
  const h = new Harness();
  h.gh.failOn.add("file");
  await h.pass(FAIL, "2026-11-18T23:00:00Z");
  let r = await h.pass(FAIL, "2026-11-18T23:10:00Z");
  assert.ok(r.errors.length);
  assert.equal(r.state.alerted, false);
  h.gh.failOn.clear();
  r = await h.pass(FAIL, "2026-11-18T23:20:00Z");
  assert.equal(r.event, "alert");
  assert.deepEqual(r.errors, []);
  assert.deepEqual(h.gh.kinds(), ["find", "file", "find", "file"]);
});

test("recovery comments once and closes a Worker-filed incident", async () => {
  const h = new Harness();
  await h.alerted();
  let r = await h.pass(OK, "2026-11-19T01:15:00Z");
  assert.equal(r.event, "recovered");
  assert.match(r.message, /after about 2h15m/);
  assert.deepEqual(h.gh.kinds(), ["find", "file", "comment", "close"]);
  assert.equal(h.gh.calls[3][1], 9001);
  r = await h.pass(OK, "2026-11-19T01:25:00Z");
  assert.equal(r.event, "ok");
  assert.equal(h.gh.calls.length, 4);
});

test("recovery leaves a hand-filed incident open", async () => {
  const h = new Harness(new FakeGitHub(5708));
  await h.alerted();
  await h.pass(OK, "2026-11-18T23:30:00Z");
  assert.deepEqual(h.gh.kinds(), ["find", "comment", "comment"]);
});

test("blip below threshold recovers quietly", async () => {
  const h = new Harness();
  await h.pass(FAIL, "2026-11-18T23:00:00Z");
  const r = await h.pass(OK, "2026-11-18T23:10:00Z");
  assert.equal(r.event, "ok");
  assert.equal(r.state.window_start, null);
  assert.deepEqual(h.gh.calls, []);
});

test("failed close is retried without a second comment", async () => {
  const h = new Harness();
  await h.alerted();
  h.gh.failOn.add("close");
  let r = await h.pass(OK, "2026-11-18T23:20:00Z");
  assert.ok(r.errors.length);
  h.gh.failOn.clear();
  r = await h.pass(OK, "2026-11-18T23:30:00Z");
  assert.equal(r.event, "recovered");
  assert.deepEqual(h.gh.kinds(), ["find", "file", "comment", "close", "close"]);
});

test("new outage after recovery alerts again", async () => {
  const h = new Harness();
  await h.alerted();
  await h.pass(OK, "2026-11-18T23:20:00Z");
  await h.pass(FAIL, "2026-11-19T02:00:00Z");
  const r = await h.pass(FAIL, "2026-11-19T02:10:00Z");
  assert.equal(r.event, "alert");
  assert.equal(r.incident, 9002);
});

test("observe mode reports but writes nothing", async () => {
  let state = null;
  for (const at of ["2026-11-18T23:00:00Z", "2026-11-18T23:10:00Z"]) {
    ({ state } = await runPass(state, FAIL, null, t(at), 2, "x"));
  }
  assert.equal(state.alerted, true);
  const r = await runPass(state, OK, null, t("2026-11-18T23:20:00Z"), 2, "x");
  assert.equal(r.event, "recovered");
  assert.ok(r.message);
});

test("healthz classification", () => {
  assert.deepEqual(classifyHealthz(200, '{"ok":true,"service":"queue-broker"}'), { ok: true, detail: "" });
  assert.equal(classifyHealthz(200, '{"ok":false}').ok, false);
  assert.equal(classifyHealthz(200, "<html>").ok, false);
  assert.match(classifyHealthz(302, "").detail, /Access refused/);
  assert.match(classifyHealthz(530, "error 1033").detail, /tunnel could not reach/);
});

test("GitHub client sends auth, UA and the incident labels", async () => {
  const seen = [];
  const fakeFetch = async (url, init) => {
    seen.push({ url, init });
    if (init.method === "GET") return new Response(JSON.stringify({ items: [{ number: 7, title: `${INCIDENT_TITLE_PREFIX} x` }] }));
    if (init.method === "POST" && url.endsWith("/issues")) return new Response(JSON.stringify({ number: 8 }), { status: 201 });
    return new Response("{}", { status: 200 });
  };
  const gh = makeGitHub("tok", "developing-today/code", fakeFetch);
  assert.equal(await gh.find(), 7);
  assert.equal(await gh.file("t", "b"), 8);
  const post = JSON.parse(seen[1].init.body);
  assert.deepEqual(post.labels, ["kind/bug", "priority/high", "area/infra", "source/fleet-monitor"]);
  assert.equal(seen[0].init.headers.Authorization, "Bearer tok");
  assert.ok(seen[0].init.headers["User-Agent"]);
  await assert.rejects(
    makeGitHub("tok", "o/r", async () => new Response("nope", { status: 403 })).close(1),
    /HTTP 403/,
  );
});
