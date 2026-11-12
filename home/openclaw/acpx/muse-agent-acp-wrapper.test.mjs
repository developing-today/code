// Tests for the acpx muse agent adapter (code#2012).
// Run: node --test home/openclaw/acpx/muse-agent-acp-wrapper.test.mjs
//
// The adapter is driven over ACP (NDJSON stdio) against a fake `muse`
// binary that speaks just enough MSP (serve) and exec JSONL to exercise
// the #1721 gates: env hygiene, pool check, turn budget, ledger, the
// terminal auth error, and the exec fallback. No pool, no network.
import assert from "node:assert/strict";
import { test } from "node:test";
import { spawn } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ADAPTER = join(dirname(fileURLToPath(import.meta.url)), "muse-agent-acp-wrapper.mjs");

const FAKE_MUSE = `#!/usr/bin/env node
import { createInterface } from "node:readline";
import { writeFileSync } from "node:fs";
const args = process.argv.slice(2);
if (process.env.FAKE_MUSE_ENV_DUMP) {
  writeFileSync(process.env.FAKE_MUSE_ENV_DUMP, JSON.stringify({
    hasMetaApiKey: "META_API_KEY" in process.env,
    args,
  }));
}
if (args[1] === "--help" || args[0] === "--help") {
  console.log(args[0] === "serve"
    ? "Usage: muse serve [--provider <P>] [--disable-request-user-input]"
    : "Usage: muse exec [--json] [--provider <MODE>] [PROMPT]");
  process.exit(0);
}
if (args[0] === "exec") {
  console.log(JSON.stringify({ payload_type: "run.output.delta", payload: { text: "exec hello" } }));
  console.log(JSON.stringify({ payload_type: "run.usage", payload: { models: [{ input_tokens: 7, output_tokens: 2 }] } }));
  console.log(JSON.stringify({ payload_type: "run.terminal.completed", payload: { terminal: "completed" } }));
  process.exit(0);
}
if (args[0] === "serve") {
  if (process.env.FAKE_MUSE_SERVE_FAIL) process.exit(1);
  const rl = createInterface({ input: process.stdin });
  rl.on("line", (line) => {
    let msg;
    try { msg = JSON.parse(line); } catch { return; }
    const send = (o) => process.stdout.write(JSON.stringify(o) + "\\n");
    if (msg.method === "initialize") {
      send({ jsonrpc: "2.0", id: msg.id, result: { serverInfo: { name: "fake-muse", version: "0" } } });
    } else if (msg.method === "session/start") {
      send({ jsonrpc: "2.0", id: msg.id, result: { session: { sessionId: "fake-session-1" } } });
    } else if (msg.method === "turn/start") {
      const sessionId = msg.params.sessionId;
      const text = msg.params.input?.[0]?.text || "";
      send({ jsonrpc: "2.0", id: msg.id, result: { turnId: "t1" } });
      setTimeout(() => {
        if (text.includes("AUTHFAIL")) {
          send({ jsonrpc: "2.0", method: "turn/completed", params: { sessionId, turnId: "t1", terminal: "failed", reason: "OAuth login expired; please login again" } });
          return;
        }
        send({ jsonrpc: "2.0", method: "item/started", params: { sessionId, item: { itemId: "i1", kind: "agentMessage" } } });
        send({ jsonrpc: "2.0", method: "item/delta", params: { sessionId, itemId: "i1", field: "text", delta: "hello " } });
        send({ jsonrpc: "2.0", method: "item/delta", params: { sessionId, itemId: "i1", field: "text", delta: "world" } });
        send({ jsonrpc: "2.0", method: "session/tokenUsage", params: { sessionId, usage: { inputTokens: 10, outputTokens: 5, cacheReadTokens: 3 } } });
        send({ jsonrpc: "2.0", method: "turn/completed", params: { sessionId, turnId: "t1", terminal: "completed" } });
      }, 10);
    } else if (msg.id !== undefined) {
      send({ jsonrpc: "2.0", id: msg.id, result: {} });
    }
  });
}
`;

function setup({ poolDoc } = {}) {
  const dir = mkdtempSync(join(tmpdir(), "muse-acp-test-"));
  const home = join(dir, "home");
  mkdirSync(join(home, ".config", "muse"), { recursive: true });
  writeFileSync(join(home, ".config", "muse", "auth.json"), "{}");
  const fakeBin = join(dir, "fake-muse.mjs");
  writeFileSync(fakeBin, FAKE_MUSE);
  if (poolDoc) writeFileSync(join(dir, "latest.json"), JSON.stringify(poolDoc));
  return { dir, home, fakeBin, envDump: join(dir, "env.json") };
}

// The adapter invokes MUSE_BIN with subcommand args; wrap the fake in a
// tiny shim so `node fake-muse.mjs serve ...` is the spawn target.
function startAdapter(opts = {}) {
  const ctx = setup(opts);
  const shim = join(ctx.dir, "muse-shim.sh");
  writeFileSync(shim, `#!/bin/sh\nexec "${process.execPath}" "${ctx.fakeBin}" "$@"\n`, { mode: 0o755 });
  const child = spawn(process.execPath, [ADAPTER], {
    env: {
      ...process.env,
      HOME: ctx.home,
      META_API_KEY: "sk-must-never-reach-the-child",
      MUSE_ACP_MUSE_BIN: shim,
      MUSE_ACP_POOL_FILE: join(ctx.dir, "latest.json"),
      MUSE_ACP_LEDGER: join(ctx.dir, "ledger.jsonl"),
      FAKE_MUSE_ENV_DUMP: ctx.envDump,
      ...(opts.adapterEnv || {}),
    },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const state = { seq: 0, pending: new Map(), updates: [] };
  let buf = "";
  child.stdout.on("data", (chunk) => {
    buf += chunk.toString();
    const lines = buf.split("\n");
    buf = lines.pop();
    for (const line of lines) {
      if (!line.trim().startsWith("{")) continue;
      const msg = JSON.parse(line);
      if (msg.id !== undefined && state.pending.has(msg.id)) {
        state.pending.get(msg.id)(msg);
        state.pending.delete(msg.id);
      } else if (msg.method === "session/update") {
        state.updates.push(msg.params.update);
      }
    }
  });
  const rpc = (method, params) =>
    new Promise((resolve) => {
      const id = ++state.seq;
      state.pending.set(id, resolve);
      child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    });
  const ledgerFile = join(ctx.dir, "ledger.jsonl");
  const ledger = () =>
    existsSync(ledgerFile) ? readFileSync(ledgerFile, "utf8").trim().split("\n").map((l) => JSON.parse(l)) : [];
  return { child, rpc, state, ledger, envDump: ctx.envDump };
}

const freshPool = (weeklyPct) => ({
  status: "ok",
  measuredAt: new Date().toISOString(),
  windows: [
    { name: "5h", usedPct: 12 },
    { name: "weekly", usedPct: weeklyPct },
  ],
});

async function newSession(ctx) {
  const init = await ctx.rpc("initialize", { protocolVersion: 1, clientCapabilities: {} });
  assert.equal(init.result.protocolVersion, 1);
  return ctx.rpc("session/new", { cwd: "/tmp", mcpServers: [] });
}

test("happy path: ACP session completes a turn; env hygiene + ledger hold", async () => {
  const ctx = startAdapter({ poolDoc: freshPool(42) });
  try {
    const created = await newSession(ctx);
    assert.equal(created.result.sessionId, "fake-session-1");
    const res = await ctx.rpc("session/prompt", {
      sessionId: "fake-session-1",
      prompt: [{ type: "text", text: "say hello" }],
    });
    assert.equal(res.result.stopReason, "end_turn");
    const text = ctx.state.updates
      .filter((u) => u.sessionUpdate === "agent_message_chunk")
      .map((u) => u.content.text)
      .join("");
    assert.equal(text, "hello world");
    const turn = ctx.ledger().find((e) => e.kind === "turn");
    assert.ok(turn, "ledger has a turn entry");
    assert.equal(turn.inputTokens, 10);
    assert.equal(turn.outputTokens, 5);
    assert.equal(turn.mode, "serve");
    const dump = JSON.parse(readFileSync(ctx.envDump, "utf8"));
    assert.equal(dump.hasMetaApiKey, false, "META_API_KEY must be deleted from the child env");
  } finally {
    ctx.child.kill();
  }
});

test("pool check: a fresh 100% reading refuses dispatch", async () => {
  const ctx = startAdapter({ poolDoc: freshPool(100) });
  try {
    await ctx.rpc("initialize", { protocolVersion: 1, clientCapabilities: {} });
    const created = await ctx.rpc("session/new", { cwd: "/tmp", mcpServers: [] });
    assert.match(created.error.message, /meta pool at capacity/);
    assert.ok(ctx.ledger().some((e) => e.kind === "refusal" && e.reason === "pool-full"));
  } finally {
    ctx.child.kill();
  }
});

test("pool check: a stale 100% reading is not a known 100% and does not refuse", async () => {
  const stale = freshPool(100);
  stale.measuredAt = new Date(Date.now() - 2 * 3_600_000).toISOString();
  const ctx = startAdapter({ poolDoc: stale });
  try {
    const created = await newSession(ctx);
    assert.equal(created.result.sessionId, "fake-session-1");
  } finally {
    ctx.child.kill();
  }
});

test("turn budget: prompts past the budget end as max_turn_requests without dispatch", async () => {
  const ctx = startAdapter({ poolDoc: freshPool(10), adapterEnv: { MUSE_ACP_TURN_BUDGET: "1" } });
  try {
    await newSession(ctx);
    const first = await ctx.rpc("session/prompt", {
      sessionId: "fake-session-1",
      prompt: [{ type: "text", text: "one" }],
    });
    assert.equal(first.result.stopReason, "end_turn");
    const second = await ctx.rpc("session/prompt", {
      sessionId: "fake-session-1",
      prompt: [{ type: "text", text: "two" }],
    });
    assert.equal(second.result.stopReason, "max_turn_requests");
    assert.equal(ctx.ledger().filter((e) => e.kind === "turn").length, 1);
    assert.ok(ctx.ledger().some((e) => e.kind === "refusal" && e.reason === "turn-budget"));
  } finally {
    ctx.child.kill();
  }
});

test("auth failure is a terminal 'muse login required on laptop' error", async () => {
  const ctx = startAdapter({ poolDoc: freshPool(10) });
  try {
    await newSession(ctx);
    const res = await ctx.rpc("session/prompt", {
      sessionId: "fake-session-1",
      prompt: [{ type: "text", text: "AUTHFAIL please" }],
    });
    assert.equal(res.error.message, "muse login required on laptop");
    const again = await ctx.rpc("session/prompt", {
      sessionId: "fake-session-1",
      prompt: [{ type: "text", text: "retry?" }],
    });
    assert.equal(again.error.message, "muse login required on laptop");
  } finally {
    ctx.child.kill();
  }
});

test("exec fallback: serve failing before commit opens an exec-backed session", async () => {
  const ctx = startAdapter({ poolDoc: freshPool(10), adapterEnv: { FAKE_MUSE_SERVE_FAIL: "1" } });
  try {
    const created = await newSession(ctx);
    assert.match(created.result.sessionId, /^exec-/);
    const res = await ctx.rpc("session/prompt", {
      sessionId: created.result.sessionId,
      prompt: [{ type: "text", text: "hi" }],
    });
    assert.equal(res.result.stopReason, "end_turn");
    const text = ctx.state.updates.map((u) => u.content?.text || "").join("");
    assert.equal(text, "exec hello");
    const turn = ctx.ledger().find((e) => e.kind === "turn");
    assert.equal(turn.mode, "exec");
    assert.equal(turn.inputTokens, 7);
  } finally {
    ctx.child.kill();
  }
});
