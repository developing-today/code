// Tests for opencode-usage-export.mjs (slice 1 of code#2072).
// Offline: fixture SQLite stores are built in temp dirs with
// node:sqlite, in the exact session_v2 / session_message shape the
// live managed-instance store carries (probed 2026-11-02, task 6610).
// Run: node --test pm-stack/opencode-usage-export/
import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { DatabaseSync } from "node:sqlite";
import { run } from "./opencode-usage-export.mjs";

const NOW = Date.parse("2026-11-02T18:00:00Z");
const HOUR = 3600000;

function makeStore(path, { sessions = [], messages = [] } = {}) {
  return (async () => {
    await mkdir(dirname(path), { recursive: true });
    const db = new DatabaseSync(path);
    db.exec(`CREATE TABLE session_v2 (
      id text PRIMARY KEY, project_id text, parent_id text, slug text,
      directory text, title text, version text, agent text, model text,
      cost real DEFAULT 0, tokens_input integer DEFAULT 0,
      tokens_output integer DEFAULT 0, tokens_reasoning integer DEFAULT 0,
      tokens_cache_read integer DEFAULT 0, tokens_cache_write integer DEFAULT 0,
      time_created integer NOT NULL, time_updated integer NOT NULL)`);
    db.exec(`CREATE TABLE session_message (
      id text PRIMARY KEY, session_id text NOT NULL, type text,
      seq integer, time_created integer NOT NULL,
      time_updated integer NOT NULL, data text NOT NULL)`);
    const insS = db.prepare(`INSERT INTO session_v2
      (id, project_id, parent_id, directory, agent, model, cost,
       tokens_input, tokens_output, tokens_reasoning,
       tokens_cache_read, tokens_cache_write, time_created, time_updated)
      VALUES (?, 'proj', ?, '/home/user/git/code', 'build', ?, ?, ?, ?, ?, ?, ?, ?, ?)`);
    for (const s of sessions) {
      insS.run(
        s.id, s.parentId ?? null,
        JSON.stringify({ id: s.model ?? "workhorse", providerID: s.provider ?? "opencode-route" }),
        s.cost ?? 0, s.tokens?.input ?? 0, s.tokens?.output ?? 0,
        s.tokens?.reasoning ?? 0, s.tokens?.cacheRead ?? 0, s.tokens?.cacheWrite ?? 0,
        s.created ?? NOW - HOUR, s.updated ?? NOW - 60000,
      );
    }
    const insM = db.prepare(`INSERT INTO session_message
      (id, session_id, type, seq, time_created, time_updated, data)
      VALUES (?, ?, ?, ?, ?, ?, ?)`);
    messages.forEach((m, i) => {
      insM.run(m.id ?? `msg_${i}`, m.sessionId, m.type ?? "assistant", i,
        NOW - 30000, NOW - 30000, JSON.stringify(m.data));
    });
    db.close();
  })();
}

const assistantMsg = (sessionId, { model = "workhorse", provider = "opencode-route", cost = 0.5, tokens } = {}) => ({
  sessionId,
  type: "assistant",
  data: {
    agent: "build",
    model: { id: model, providerID: provider },
    cost,
    finish: "stop",
    tokens: tokens ?? { input: 100, output: 10, reasoning: 2, cache: { read: 40, write: 3 } },
    time: { created: NOW - 30000, completed: NOW - 29000 },
  },
});

async function fakeHome() {
  const home = await mkdtemp(join(tmpdir(), "oc-usage-home-"));
  const outDir = await mkdtemp(join(tmpdir(), "oc-usage-out-"));
  return { home, outDir };
}
const defaultDb = (home) => join(home, ".local/share/opencode/opencode.db");
const chamberDb = (home) => join(home, ".local/share/openchamber/opencode.db");

async function assertInvalidHoursOption(option, valueArgs) {
  const { home, outDir } = await fakeHome();
  const { exitCode, doc } = await run({
    home,
    now: NOW,
    argv: [...valueArgs, "--out-dir", outDir],
  });
  assert.notEqual(exitCode, 0);
  assert.equal(doc.status, "invalid-argument");
  assert.match(doc.detail, new RegExp(option));
  assert.equal(doc.sessions, undefined);
  const latest = JSON.parse(await readFile(join(outDir, "latest.json"), "utf8"));
  assert.equal(latest.status, "invalid-argument");
  assert.match(latest.detail, new RegExp(option));
}

for (const option of ["--window-hours", "--fresh-hours"]) {
  for (const value of ["banana", "NaN", "Infinity", "-Infinity", "0", "-1"]) {
    test(`${option} rejects invalid value ${value} before reading a store`, async () => {
      await assertInvalidHoursOption(option, [option, value]);
    });
  }
  test(`${option} rejects a missing value before the next option`, async () => {
    await assertInvalidHoursOption(option, [option]);
  });
}

test("positive finite hour options preserve normal export behavior", async () => {
  const { home, outDir } = await fakeHome();
  await makeStore(defaultDb(home), {
    sessions: [
      { id: "ses_recent" },
      { id: "ses_old", updated: NOW - 48 * HOUR, created: NOW - 49 * HOUR },
    ],
  });
  const { exitCode, doc } = await run({
    home,
    now: NOW,
    argv: ["--out-dir", outDir, "--window-hours", "12", "--fresh-hours", "24"],
  });
  assert.equal(exitCode, 0);
  assert.equal(doc.status, "ok");
  assert.equal(doc.windowHours, 12);
  assert.deepEqual(doc.sessions.map((session) => session.id), ["ses_recent"]);
});

test("fresh export covers a known session with model, tokens, cost, parentage", async () => {
  const { home, outDir } = await fakeHome();
  await makeStore(defaultDb(home), {
    sessions: [
      { id: "ses_parent", cost: 1.25, tokens: { input: 1000, output: 50, reasoning: 5, cacheRead: 400, cacheWrite: 9 } },
      { id: "ses_child", parentId: "ses_parent", model: "muse-spark-1.3-contributor", provider: "muse-direct", cost: 0.25, tokens: { input: 200, output: 20, reasoning: 0, cacheRead: 10, cacheWrite: 0 } },
      { id: "ses_old", updated: NOW - 48 * HOUR }, // outside the 24h window
    ],
    messages: [
      assistantMsg("ses_parent"),
      assistantMsg("ses_parent", { cost: 0.75 }),
      { sessionId: "ses_parent", type: "user", data: { role: "user", time: { created: NOW - 31000 } } },
      assistantMsg("ses_child", { model: "muse-spark-1.3-contributor", provider: "muse-direct", cost: 0.25 }),
    ],
  });
  const { exitCode, doc } = await run({ home, now: NOW, argv: ["--out-dir", outDir] });
  assert.equal(exitCode, 0);
  assert.equal(doc.status, "ok");
  assert.equal(doc.storePath, defaultDb(home));
  assert.equal(doc.moved, null);
  assert.equal(doc.sessions.length, 2); // ses_old is outside the window
  const parent = doc.sessions.find((s) => s.id === "ses_parent");
  const child = doc.sessions.find((s) => s.id === "ses_child");
  assert.equal(parent.parentId, null);
  assert.equal(child.parentId, "ses_parent");
  assert.deepEqual(parent.model, { id: "workhorse", providerID: "opencode-route" });
  assert.equal(parent.messages.assistant, 2);
  assert.equal(parent.messages.total, 3);
  assert.equal(parent.messages.byModel.length, 1);
  assert.equal(parent.messages.byModel[0].count, 2);
  assert.equal(parent.messages.byModel[0].cost, 1.25);
  assert.deepEqual(parent.messages.byModel[0].tokens, { input: 200, output: 20, reasoning: 4, cacheRead: 80, cacheWrite: 6 });
  assert.equal(doc.totals.sessions, 2);
  assert.equal(doc.totals.childSessions, 1);
  assert.equal(doc.totals.cost, 1.5);
  assert.equal(doc.totals.tokens.input, 1200);
  const latest = JSON.parse(await readFile(join(outDir, "latest.json"), "utf8"));
  assert.equal(latest.status, "ok");
  const state = JSON.parse(await readFile(join(outDir, "state.json"), "utf8"));
  assert.equal(state.storePath, defaultDb(home));
});

test("move detection: stale default store, fresh chamber store wins and the move is surfaced", async () => {
  const { home, outDir } = await fakeHome();
  await makeStore(defaultDb(home), {
    sessions: [{ id: "ses_stale", updated: NOW - 30 * 24 * HOUR, created: NOW - 31 * 24 * HOUR }],
  });
  await makeStore(chamberDb(home), {
    sessions: [{ id: "ses_live", cost: 2 }],
    messages: [assistantMsg("ses_live", { cost: 2 })],
  });
  // Previous ticks recorded the default path as the store.
  await writeFile(join(outDir, "state.json"), JSON.stringify({ storePath: defaultDb(home) }));
  const { exitCode, doc } = await run({ home, now: NOW, argv: ["--out-dir", outDir] });
  assert.equal(exitCode, 0);
  assert.equal(doc.status, "ok");
  assert.equal(doc.storePath, chamberDb(home));
  assert.deepEqual(doc.moved, { from: defaultDb(home), to: chamberDb(home) });
  assert.equal(doc.sessions.length, 1);
  assert.equal(doc.sessions[0].id, "ses_live");
});

test("all candidates stale: store-stale failure document, no sessions, exit 4", async () => {
  const { home, outDir } = await fakeHome();
  await makeStore(defaultDb(home), {
    sessions: [{ id: "ses_stale", updated: NOW - 30 * 24 * HOUR, created: NOW - 31 * 24 * HOUR }],
  });
  const { exitCode, doc } = await run({ home, now: NOW, argv: ["--out-dir", outDir] });
  assert.equal(exitCode, 4);
  assert.equal(doc.status, "store-stale");
  assert.equal(doc.sessions, undefined);
  assert.match(doc.detail, /may have moved/);
});

test("no candidate exists: store-missing failure document, exit 3", async () => {
  const { home, outDir } = await fakeHome();
  const { exitCode, doc } = await run({ home, now: NOW, argv: ["--out-dir", outDir] });
  assert.equal(exitCode, 3);
  assert.equal(doc.status, "store-missing");
  assert.equal(doc.sessions, undefined);
});
