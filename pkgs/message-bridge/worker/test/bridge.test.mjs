import assert from "node:assert/strict";
import { test } from "node:test";
import { matchTokenPath, safeEqual } from "../src/auth.js";
import { MemoryStore } from "../src/messages.js";
import { OWNER_COOKIE } from "../src/oauth.js";
import { DEFAULT_CONSOLE_URL, route } from "../src/routes.js";

const TOKEN = "t".repeat(20) + "0123456789abcdefXYZ_-";
const env = { ACCESS_TOKEN: TOKEN };
const BASE = "https://bridge.example";

function harness() {
  const store = new MemoryStore(() => "2026-11-19T00:00:00.000Z");
  const call = (path, init) => route(new Request(BASE + path, init), env, () => store);
  let rpcId = 0;
  const rpc = async (method, params) => {
    const res = await call(`/mcp/${TOKEN}`, {
      method: "POST",
      headers: { "content-type": "application/json", accept: "application/json, text/event-stream" },
      body: JSON.stringify({ jsonrpc: "2.0", id: ++rpcId, method, params }),
    });
    assert.equal(res.status, 200);
    assert.match(res.headers.get("content-type"), /application\/json/);
    return res.json();
  };
  const tool = async (name, args) => (await rpc("tools/call", { name, arguments: args })).result;
  return { store, call, rpc, tool };
}

test("safeEqual and token paths", () => {
  assert.ok(safeEqual("abc", "abc"));
  assert.ok(!safeEqual("abc", "abd"));
  assert.ok(!safeEqual("abc", "abcd"));
  assert.deepEqual(matchTokenPath(`/api/${TOKEN}/messages`, env), {
    area: "api",
    rest: ["messages"],
    role: "owner",
    channel: "claude",
  });
  assert.equal(matchTokenPath(`/api/wrong/messages`, env), null);
  assert.equal(matchTokenPath(`/mcp/${TOKEN}`, { ACCESS_TOKEN: "short" }), null, "short token disables access");
  assert.equal(matchTokenPath(`/mcp/${TOKEN}`, {}), null, "no token disables access");
});

test("wrong or missing token is a plain 404 everywhere", async () => {
  const { call } = harness();
  // The bare address ("/", "/mcp") asks for OAuth instead: oauth.test.mjs.
  for (const path of ["/mcp/nope", "/p/nope", "/api/nope/messages", `/x/${TOKEN}`]) {
    assert.equal((await call(path, { method: "POST", body: "{}" })).status, 404, path);
    if (path.startsWith("/x/")) {
      // A two-segment GET is the public note signal (notes.js), even
      // when its second segment happens to be a token: same bare UUID.
      const signal = await call(path);
      assert.equal(signal.status, 200, path);
      assert.match((await signal.text()).trim(), /^[0-9a-f-]{36}$/);
    } else {
      assert.equal((await call(path)).status, 404, path);
    }
  }
  const health = await (await call("/healthz")).json();
  const { tokenFingerprint } = await import("../src/auth.js");
  assert.deepEqual(health, {
    ok: true,
    configured: true,
    chatgpt_configured: false,
    version: null,
    token_fingerprint: await tokenFingerprint(TOKEN),
  });
  assert.doesNotMatch(JSON.stringify(health), new RegExp(TOKEN));
});

test("token fingerprint is short, stable and token-specific", async () => {
  const { tokenFingerprint } = await import("../src/auth.js");
  const a = await tokenFingerprint(TOKEN);
  assert.match(a, /^[0-9a-f]{16}$/);
  assert.equal(await tokenFingerprint(TOKEN), a);
  assert.notEqual(await tokenFingerprint(`${TOKEN}x`), a);
  const versioned = { ...env, CF_VERSION_METADATA: { id: "v-123" } };
  const res = await route(new Request(`${BASE}/healthz`), versioned, () => null);
  assert.equal((await res.json()).version, "v-123");
});

test("MCP initialize, tools/list, notifications, unknown method", async () => {
  const { rpc, call } = harness();
  const init = await rpc("initialize", {
    protocolVersion: "2025-06-18",
    capabilities: {},
    clientInfo: { name: "test", version: "1" },
  });
  assert.equal(init.result.protocolVersion, "2025-06-18");
  assert.equal(init.result.serverInfo.name, "message-bridge");
  assert.ok(init.result.capabilities.tools);
  const odd = await rpc("initialize", { protocolVersion: "1999-01-01" });
  assert.equal(odd.result.protocolVersion, "2025-11-25");

  const notif = await call(`/mcp/${TOKEN}`, {
    method: "POST",
    body: JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }),
  });
  assert.equal(notif.status, 202);

  const tools = await rpc("tools/list", {});
  assert.deepEqual(
    tools.result.tools.map((t) => t.name),
    ["send_message", "read_messages", "ack_messages"],
  );
  assert.equal((await rpc("nope/nope", {})).error.code, -32601);
  assert.equal((await rpc("ping")).result && true, true);

  const get = await call(`/mcp/${TOKEN}`);
  assert.equal(get.status, 405);
  const bad = await call(`/mcp/${TOKEN}`, { method: "POST", body: "{not json" });
  assert.equal(bad.status, 400);
  assert.equal((await bad.json()).error.code, -32700);
});

test("send, receive, mark read and ack through MCP; the API shares the store", async () => {
  const { tool, call, store } = harness();
  const sent = await tool("send_message", { body: "hello from claude", sender: "claude" });
  assert.equal(sent.isError, undefined);
  assert.equal(sent.structuredContent.message.direction, "outbound");
  assert.equal(sent.structuredContent.message.source, "mcp");

  // JSON API posts -> inbound via api.
  const first = await call(`/api/${TOKEN}/messages`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ body: "hi claude <b>", sender: "Drew" }),
  });
  assert.equal(first.status, 201);
  const api = await call(`/api/${TOKEN}/messages`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ body: "from a script" }),
  });
  assert.equal(api.status, 201);
  assert.equal((await api.json()).message.source, "api");

  const unread = await tool("read_messages", { unread_only: true });
  assert.deepEqual(
    unread.structuredContent.messages.map((m) => [m.body, m.source]),
    [
      ["hi claude <b>", "api"],
      ["from a script", "api"],
    ],
  );
  assert.equal(unread.structuredContent.stats.unread_inbound, 2);
  assert.equal(unread.structuredContent.next_since_id, 3);

  const marked = await tool("read_messages", { unread_only: true, limit: 1, mark_read: true });
  assert.equal(marked.structuredContent.acked, 1);
  assert.equal(marked.structuredContent.stats.unread_inbound, 1);

  const acked = await tool("ack_messages", { up_to_id: 3 });
  assert.equal(acked.structuredContent.acked, 1);
  assert.equal(acked.structuredContent.stats.unread_inbound, 0);

  const outbound = await tool("read_messages", { direction: "outbound" });
  assert.equal(outbound.structuredContent.messages.length, 1);
  const after = await tool("read_messages", { direction: "all", since_id: 1 });
  assert.deepEqual(
    after.structuredContent.messages.map((m) => m.id),
    [2, 3],
  );

  // History JSON, newest first.
  const hist = await (await call(`/api/${TOKEN}/messages?order=desc`)).json();
  assert.deepEqual(
    hist.messages.map((m) => m.id),
    [3, 2, 1],
  );
  assert.equal(hist.stats.total, 3);

  assert.equal(store.rows.length, 3);
});

test("the owner link sets the cookie and redirects to the console (#14378)", async () => {
  const { call } = harness();
  // The retired page's address is now a pointer: no messaging UI, just
  // the owner cookie and a redirect to the console's Messages surface.
  const link = await call(`/p/${TOKEN}`);
  assert.equal(link.status, 303);
  assert.equal(link.headers.get("location"), DEFAULT_CONSOLE_URL);
  assert.match(link.headers.get("set-cookie"), new RegExp(`^${OWNER_COOKIE}=[^;]+; Path=/; .*Secure; HttpOnly; SameSite=Lax`));
  assert.equal(link.headers.get("cache-control"), "no-store");
  assert.equal(link.headers.get("referrer-policy"), "no-referrer");
  assert.ok(!(await link.text()).includes(TOKEN), "the redirect carries no token");
  // Query strings from the old page (paging) change nothing.
  assert.equal((await call(`/p/${TOKEN}?before_id=6`)).headers.get("location"), DEFAULT_CONSOLE_URL);
  // Form posts went with the page.
  for (const method of ["POST", "PUT", "DELETE"]) {
    assert.equal((await call(`/p/${TOKEN}`, { method, body: "body=x" })).status, 405, method);
  }
  // The console address is the CONSOLE_URL var when one is set.
  const store = new MemoryStore();
  const custom = await route(
    new Request(`${BASE}/p/${TOKEN}`),
    { ...env, CONSOLE_URL: "https://console.example/console/#more/messages" },
    () => store,
  );
  assert.equal(custom.headers.get("location"), "https://console.example/console/#more/messages");
});

test("bad input is a tool error or a 400, never stored", async () => {
  const { tool, call, store } = harness();
  const empty = await tool("send_message", { body: "   " });
  assert.equal(empty.isError, true);
  const extra = await tool("send_message", { body: "x", colour: "red" });
  assert.equal(extra.isError, true);
  for (const bad of [{ to: "not a name!" }, { to: "claude" }, { to: "channel:nobody" }, { to: "drew", kind: "instruction" }, { kind: "shout" }, { to: [] , kind: "instruction" }]) {
    assert.equal((await tool("send_message", { body: "x", ...bad })).isError, true, JSON.stringify(bad));
  }
  const big = await tool("send_message", { body: "x".repeat(16001) });
  assert.equal(big.isError, true);
  const badDir = await tool("read_messages", { direction: "sideways" });
  assert.equal(badDir.isError, true);
  const noSel = await tool("ack_messages", {});
  assert.equal(noSel.isError, true);
  const r = await call(`/api/${TOKEN}/messages`, { method: "POST", body: "nope" });
  assert.equal(r.status, 400);
  const r2 = await call(`/api/${TOKEN}/messages?limit=0`);
  assert.equal(r2.status, 400);
  assert.equal(store.rows.length, 0);
});

test("history reads back past the newest page with before_id", async () => {
  const { call, store } = harness();
  for (let i = 1; i <= 205; i++) {
    await store.add({ body: `m${i}`, direction: i % 2 ? "inbound" : "outbound", source: i % 2 ? "api" : "mcp" });
  }
  const api = await (await call(`/api/${TOKEN}/messages?order=desc&limit=3&before_id=100`)).json();
  assert.deepEqual(
    api.messages.map((m) => m.id),
    [99, 98, 97],
  );
  assert.equal((await call(`/api/${TOKEN}/messages?before_id=0`)).status, 400);
});

test("deploy argument parsing fails closed", async () => {
  const { parseArgs } = await import("../deploy-args.mjs");
  const id = "02854b08-81e8-4ee5-bbc4-f7cba767fa8f";
  assert.deepEqual(parseArgs([]), { mode: "deploy", dryRun: false, deployVersion: null, allowSecretChange: false });
  assert.deepEqual(parseArgs(["--dry-run"]), { mode: "deploy", dryRun: true, deployVersion: null, allowSecretChange: false });
  assert.equal(parseArgs(["--code-only"]).mode, "code-only");
  assert.equal(parseArgs(["--print-urls"]).mode, "print-urls");
  assert.deepEqual(parseArgs(["--deploy-version", id, "--dry-run"]), {
    mode: "deploy-version",
    dryRun: true,
    deployVersion: id,
    allowSecretChange: false,
  });
  assert.deepEqual(parseArgs(["--dry-run", "--deploy-version", id]), {
    mode: "deploy-version",
    dryRun: true,
    deployVersion: id,
    allowSecretChange: false,
  });
  assert.equal(parseArgs(["--deploy-version", id, "--allow-secret-change"]).allowSecretChange, true);
  for (const bad of [
    ["deploy"],
    ["--nope"],
    ["--dry-run", "--dry-run"],
    ["--dry-run", "--print-urls"],
    ["--code-only", "--dry-run"],
    ["--deploy-version"],
    ["--deploy-version", "latest"],
    ["--deploy-version", id, "--code-only"],
    ["--allow-secret-change"],
    ["--dry-run", "--allow-secret-change"],
    ["--deploy-version", id, "--allow-secret-change", "--allow-secret-change"],
  ]) {
    assert.throws(() => parseArgs(bad), undefined, bad.join(" "));
  }
});

test("worker existence probe bootstraps only on the API's not-found answer", async () => {
  const { classifyWorkerProbe } = await import("../deploy-probe.mjs");
  assert.equal(classifyWorkerProbe(200, { success: true, result: {} }), true);
  assert.equal(
    classifyWorkerProbe(404, { success: false, errors: [{ code: 10007, message: "This Worker does not exist on your account." }] }),
    false,
  );
  for (const [status, body] of [
    [401, { success: false, errors: [{ code: 10000, message: "Authentication error" }] }],
    [403, { success: false, errors: [{ code: 9109, message: "Unauthorized" }] }],
    [404, { success: false, errors: [{ code: 7003, message: "Could not route" }] }],
    [404, null],
    [500, null],
    [200, { success: false }],
  ]) {
    assert.throws(() => classifyWorkerProbe(status, body), /aborting before any upload/, `${status}`);
  }
});

test("code-only deploys refuse to inherit a token from a full deploy newer than the live version", async () => {
  const { strayBeforeCodeOnly, VersionListGap } = await import("../deploy-probe.mjs");
  const v = (id, number, how) => ({ id, number, annotations: { "workers/message": `deploy abc via ${how} (run 1)` } });
  const code = "message-bridge-deploy workflow";
  const full = "nix run .#message-bridge-deploy";
  // Before the upload: nothing newer than live, or only code-only uploads (in any order).
  assert.equal(strayBeforeCodeOnly([v("live", 5, full)], "live"), null);
  assert.equal(strayBeforeCodeOnly([v("c1", 6, code), v("live", 5, full), v("c2", 7, code)], "live"), null);
  assert.equal(strayBeforeCodeOnly([v("c1", 7, code), v("f1", 6, full), v("live", 5, code)], "live")?.id, "f1");
  // After the upload: a full deploy that slipped in before ours counts, one after it doesn't.
  assert.equal(strayBeforeCodeOnly([v("mine", 7, code), v("f1", 6, full), v("live", 5, code)], "live", "mine")?.id, "f1");
  assert.equal(strayBeforeCodeOnly([v("f1", 7, full), v("mine", 6, code), v("live", 5, code)], "live", "mine"), null);
  // A version the list hasn't caught up with yet could be a full deploy: no verdict on a gap.
  assert.throws(() => strayBeforeCodeOnly([v("mine", 7, code), v("live", 5, code)], "live", "mine"), VersionListGap);
  assert.throws(() => strayBeforeCodeOnly([v("c2", 7, code), v("live", 5, code)], "live"), VersionListGap);
  // This run's own upload not listed yet is list lag, so it is retried.
  assert.throws(() => strayBeforeCodeOnly([v("live", 5, code)], "live", "mine"), VersionListGap);
  assert.throws(() => strayBeforeCodeOnly([v("x", 1, code)], "live"), /not in the versions list/);
  assert.throws(() => strayBeforeCodeOnly([v("live", 6, code), v("mine", 5, code)], "live", "mine"), /not newer/);
  assert.throws(() => strayBeforeCodeOnly([{ id: "live" }], "live"), /no version number/);
});

test("a code-only deploy needs the live token fingerprint", async () => {
  const { codeOnlyFingerprint } = await import("../deploy-probe.mjs");
  assert.equal(codeOnlyFingerprint({ ok: true, configured: true, token_fingerprint: "0123456789abcdef" }), "0123456789abcdef");
  assert.throws(() => codeOnlyFingerprint(null), /could not read/);
  assert.throws(() => codeOnlyFingerprint({ ok: true, configured: true }), /no token fingerprint/);
  assert.throws(() => codeOnlyFingerprint({ ok: true, token_fingerprint: null }), /no token fingerprint/);
});

test("a failed promotion or verification puts the previous version back", async () => {
  const { promoteOrRollBack } = await import("../deploy-probe.mjs");
  const run = async ({ promoteFails = false, verifyFails = false, rollbackFails = false, previousId = "old" }) => {
    const log = [];
    const result = await promoteOrRollBack({
      newId: "new",
      previousId,
      promote: () => {
        log.push("promote");
        if (promoteFails) throw new Error("promote failed");
      },
      verify: async () => {
        log.push("verify");
        if (verifyFails) throw new Error("verify timed out");
      },
      rollback: (id) => {
        log.push(`rollback ${id}`);
        if (rollbackFails) throw new Error("api down");
      },
    }).then(
      () => "ok",
      (err) => err.message,
    );
    return { log, result };
  };
  assert.deepEqual(await run({}), { log: ["promote", "verify"], result: "ok" });
  const timedOut = await run({ verifyFails: true });
  assert.deepEqual(timedOut.log, ["promote", "verify", "rollback old"]);
  assert.match(timedOut.result, /verify timed out; version old was put back live/);
  const promoteErr = await run({ promoteFails: true });
  assert.deepEqual(promoteErr.log, ["promote", "rollback old"]);
  const both = await run({ verifyFails: true, rollbackFails: true });
  assert.match(both.result, /failed too \(api down\).*--deploy-version old --allow-secret-change/);
  const nothingBefore = await run({ previousId: null });
  assert.deepEqual(nothingBefore.log, []);
  assert.match(nothingBefore.result, /refusing to promote/);
});

test("a promotion that replaced a different version than the one checked goes back to that one", async () => {
  const { promoteOrRollBack, replacedOtherThan } = await import("../deploy-probe.mjs");
  const d = (id) => ({ versions: [{ version_id: id, percentage: 100 }] });
  assert.equal(replacedOtherThan([d("new"), d("old"), d("older")], "new", "old"), null);
  assert.equal(replacedOtherThan([d("new"), d("rotated"), d("old")], "new", "old"), "rotated");
  assert.throws(() => replacedOtherThan([d("old")], "new", "old"), /newest deployment is not/);
  assert.throws(() => replacedOtherThan([d("new")], "new", "old"), /could not tell/);
  const log = [];
  const err = await promoteOrRollBack({
    newId: "new",
    previousId: "old",
    promote: () => log.push("promote"),
    verify: async () => {
      const e = new Error("another deployment went live");
      e.rollbackTo = "rotated";
      throw e;
    },
    rollback: (id) => log.push(`rollback ${id}`),
  }).catch((e) => e);
  assert.deepEqual(log, ["promote", "rollback rotated"]);
  assert.match(err.message, /version rotated was put back live/);
});

test("a failed verification never rolls back over a deployment made after it", async () => {
  const { promoteOrRollBack } = await import("../deploy-probe.mjs");
  const log = [];
  const steps = (stillNewest) => ({
    newId: "v2",
    previousId: "v1",
    promote: () => log.push("promote v2"),
    verify: async () => {
      log.push("v3 deployed by someone else");
      throw new Error("verify timed out");
    },
    rollback: (id) => log.push(`rollback ${id}`),
    stillNewest,
  });
  const err = await promoteOrRollBack(steps(async () => false)).catch((e) => e);
  assert.deepEqual(log, ["promote v2", "v3 deployed by someone else"]);
  assert.match(err.message, /not rolling back/);
  log.length = 0;
  const unreadable = await promoteOrRollBack(
    steps(async () => {
      throw new Error("api down");
    }),
  ).catch((e) => e);
  assert.ok(!log.some((l) => l.startsWith("rollback")));
  assert.match(unreadable.message, /not rolling back/);
});

test("deploy.mjs rejects undeclared arguments before touching credentials", async () => {
  const { spawnSync } = await import("node:child_process");
  const { fileURLToPath } = await import("node:url");
  const script = fileURLToPath(new URL("../deploy.mjs", import.meta.url));
  for (const argv of [["deploy"], ["--nope"], ["--dry-run", "--print-urls"], ["--deploy-version", "x"]]) {
    const r = spawnSync(process.execPath, [script, ...argv], {
      encoding: "utf8",
      env: { PATH: "/nonexistent" }, // bws unreachable: any credential use would fail differently
    });
    assert.equal(r.status, 2, `${argv.join(" ")}: ${r.stderr}`);
    assert.match(r.stderr, /usage: deploy\.mjs/);
  }
  const ok = spawnSync(process.execPath, [script, "--code-only"], {
    encoding: "utf8",
    env: { PATH: "/nonexistent", MB_CHECK_ARGS_ONLY: "1" },
  });
  assert.equal(ok.status, 0, ok.stderr);
});

test("deploy.mjs --code-only needs the Cloudflare env and nothing from Bitwarden", async () => {
  const { spawnSync } = await import("node:child_process");
  const { fileURLToPath } = await import("node:url");
  const script = fileURLToPath(new URL("../deploy.mjs", import.meta.url));
  const r = spawnSync(process.execPath, [script, "--code-only"], { encoding: "utf8", env: { PATH: "/nonexistent" } });
  assert.notEqual(r.status, 0);
  assert.match(r.stderr, /CLOUDFLARE_API_TOKEN is not set/);
  assert.doesNotMatch(r.stderr, /bws/);
});

test("cloudflare.config.ts matches wrangler.jsonc", async () => {
  const { readFileSync } = await import("node:fs");
  const cfg = readFileSync(new URL("../cloudflare.config.ts", import.meta.url), "utf8");
  const wr = readFileSync(new URL("../wrangler.jsonc", import.meta.url), "utf8");
  const pick = (re, text) => text.match(re)?.[1];
  assert.equal(pick(/name: "([^"]+)"/, cfg), pick(/"name": "([^"]+)"/, wr));
  assert.equal(pick(/compatibilityDate: "([^"]+)"/, cfg), pick(/"compatibility_date": "([^"]+)"/, wr));
  assert.equal(pick(/PUBLIC_URL: bindings\.text\("([^"]+)"\)/, cfg), pick(/"PUBLIC_URL": "([^"]+)"/, wr));
  assert.equal(pick(/WORKERS_DEV_URL: bindings\.text\("([^"]+)"\)/, cfg), pick(/"WORKERS_DEV_URL": "([^"]+)"/, wr));
  assert.equal(pick(/CONSOLE_URL: bindings\.text\("([^"]+)"\)/, cfg), pick(/"CONSOLE_URL": "([^"]+)"/, wr));
  assert.ok(cfg.includes('MESSAGES: bindings.durableObject({ worker: "message-bridge", exportName: "MessageStore" })'));
  assert.ok(wr.includes('"name": "MESSAGES", "class_name": "MessageStore"'));
});

test("isOwnDeployment accepts only this run's single-version deployment", async () => {
  const { isOwnDeployment } = await import("../deploy-probe.mjs");
  const mine = { id: "d1", versions: [{ version_id: "v2", percentage: 100 }] };
  assert.equal(isOwnDeployment(mine, "v2"), true);
  assert.equal(isOwnDeployment(mine, "v2", "d1"), true);
  // the same version deployed again by someone else
  assert.equal(isOwnDeployment({ ...mine, id: "d2" }, "v2", "d1"), false);
  // a newer split deployment that still includes this version
  const split = {
    id: "d2",
    versions: [
      { version_id: "v2", percentage: 50 },
      { version_id: "v3", percentage: 50 },
    ],
  };
  assert.equal(isOwnDeployment(split, "v2"), false);
  assert.equal(isOwnDeployment({ id: "d1", versions: [{ version_id: "v1", percentage: 100 }] }, "v2"), false);
  assert.equal(isOwnDeployment(undefined, "v2"), false);
});

test("healthVersionProblem rejects a different reported version", async () => {
  const { healthVersionProblem } = await import("../deploy-probe.mjs");
  assert.equal(healthVersionProblem({ version: "v2" }, "v2"), null);
  assert.equal(healthVersionProblem({ version: "v2" }, "v2", { requireVersion: true }), null);
  // older Workers report no version: fine for a rollback, not for a new deploy
  assert.equal(healthVersionProblem({ version: null }, "v1"), null);
  assert.match(healthVersionProblem({ version: null }, "v1", { requireVersion: true }), /never answered/);
  assert.match(healthVersionProblem(null, "v1", { requireVersion: true }), /never answered/);
  // a different, configured version is wrong either way
  assert.match(healthVersionProblem({ version: "v3" }, "v1"), /answered from version v3, not v1/);
  assert.match(healthVersionProblem({ version: "v3" }, "v1", { requireVersion: true }), /v3/);
});

const GPT = "g".repeat(24) + "chatgpt-token-0123456789";

function twoChannels() {
  const env2 = { ACCESS_TOKEN: TOKEN, CHATGPT_TOKEN: GPT };
  const store = new MemoryStore(() => "2026-11-19T00:00:00.000Z");
  const call = (path, init) => route(new Request(BASE + path, init), env2, () => store);
  let rpcId = 0;
  const tool = async (token, name, args) => {
    const res = await call(`/mcp/${token}`, {
      method: "POST",
      body: JSON.stringify({ jsonrpc: "2.0", id: ++rpcId, method: "tools/call", params: { name, arguments: args } }),
    });
    assert.equal(res.status, 200);
    return (await res.json()).result;
  };
  return { env2, store, call, tool };
}

test("the ChatGPT token opens only its own MCP channel", async () => {
  const { env2, call } = twoChannels();
  assert.deepEqual(matchTokenPath(`/mcp/${GPT}`, env2), { area: "mcp", rest: [], role: "chatgpt", channel: "chatgpt" });
  for (const path of [`/p/${GPT}`, `/api/${GPT}/messages`, `/api/${GPT}/ack`]) {
    assert.equal((await call(path)).status, 404, path);
    assert.equal((await call(path, { method: "POST", body: "{}" })).status, 404, path);
  }
  // A ChatGPT token equal to the owner's is ignored rather than trusted.
  assert.equal(matchTokenPath(`/mcp/${TOKEN}`, { ACCESS_TOKEN: TOKEN, CHATGPT_TOKEN: TOKEN }).role, "owner");
  const health = await (await call("/healthz")).json();
  assert.equal(health.chatgpt_configured, true);
  assert.doesNotMatch(JSON.stringify(health), new RegExp(GPT));
  const init = await call(`/mcp/${GPT}`, {
    method: "POST",
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize", params: {} }),
  });
  assert.match((await init.json()).result.instructions, /between ChatGPT, Drew/);
  const disc = await call(`/mcp/${GPT}`, {
    method: "POST",
    body: JSON.stringify({ jsonrpc: "2.0", id: 2, method: "server/discover" }),
  });
  assert.ok((await disc.json()).result.supportedVersions.includes("2026-07-28"));
});

test("the API sends to one or both; each assistant sees and acks only its own", async () => {
  const { call, tool } = twoChannels();
  const both = await call(`/api/${TOKEN}/messages`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ body: "to both", to: ["claude", "chatgpt"] }),
  });
  assert.equal(both.status, 201);
  const onlyGpt = await call(`/api/${TOKEN}/messages`, {
    method: "POST",
    body: JSON.stringify({ body: "gpt only", to: ["chatgpt"], source: "page" }),
  });
  assert.equal(onlyGpt.status, 201);
  const made = await onlyGpt.json();
  assert.equal(made.message.channel, "chatgpt");
  assert.equal(made.messages.length, 1);
  const defaultTo = await (await call(`/api/${TOKEN}/messages`, { method: "POST", body: JSON.stringify({ body: "old caller" }) })).json();
  assert.deepEqual(defaultTo.messages.map((m) => m.channel), ["claude"]);
  for (const bad of [[], ["bard"], 7, "claude,nope"]) {
    const r = await call(`/api/${TOKEN}/messages`, { method: "POST", body: JSON.stringify({ body: "x", to: bad }) });
    assert.equal(r.status, 400, JSON.stringify(bad));
  }

  const gptRead = (await tool(GPT, "read_messages", { unread_only: true, mark_read: true })).structuredContent;
  assert.deepEqual(gptRead.messages.map((m) => m.body), ["to both", "gpt only"]);
  assert.equal(gptRead.acked, 2);
  assert.equal(gptRead.stats.unread_inbound, 0);
  const sent = (await tool(GPT, "send_message", { body: "hi from chatgpt" })).structuredContent.message;
  assert.equal(sent.channel, "chatgpt");

  const claudeRead = (await tool(TOKEN, "read_messages", { unread_only: true })).structuredContent;
  assert.deepEqual(claudeRead.messages.map((m) => m.body), ["to both", "old caller"]);
  // Claude can't ack ChatGPT's messages by id, and vice versa.
  const gptIds = gptRead.messages.map((m) => m.id);
  assert.equal((await tool(TOKEN, "ack_messages", { ids: gptIds })).structuredContent.acked, 0);
  const claudeIds = claudeRead.messages.map((m) => m.id);
  assert.equal((await tool(GPT, "ack_messages", { ids: claudeIds })).structuredContent.acked, 0);
  assert.equal((await tool(GPT, "read_messages", { direction: "all" })).structuredContent.messages.length, 3);

  const all = await (await call(`/api/${TOKEN}/messages?direction=all`)).json();
  assert.equal(all.messages.length, 5);
  assert.equal(all.stats.total, 5);
  const gptOnly = await (await call(`/api/${TOKEN}/messages?channel=chatgpt`)).json();
  assert.equal(gptOnly.messages.length, 3);
});

test("instructions target one or more agents, and agents never see their sender", async () => {
  const { call, tool } = twoChannels();
  const sent = await call(`/api/${TOKEN}/messages`, {
    method: "POST",
    body: JSON.stringify({ body: "rebase onto main", sender: "Muse", to: ["claude", "chatgpt"], kind: "instruction" }),
  });
  assert.equal(sent.status, 201);
  const made = await sent.json();
  assert.deepEqual(made.messages.map((m) => [m.channel, m.kind, m.sender]), [
    ["claude", "instruction", "Muse"],
    ["chatgpt", "instruction", "Muse"],
  ]);
  const form = await call(`/api/${TOKEN}/messages`, {
    method: "POST",
    body: JSON.stringify({ body: "from the api", sender: "Aster", to: ["chatgpt"], kind: "instruction" }),
    headers: { "content-type": "application/json" },
  });
  assert.equal(form.status, 201);
  await call(`/api/${TOKEN}/messages`, {
    method: "POST",
    body: JSON.stringify({ body: "plain note", sender: "Drew", to: "chatgpt" }),
  });

  for (const [token, count] of [[TOKEN, 1], [GPT, 2]]) {
    const read = (await tool(token, "read_messages", { kind: "instruction" })).structuredContent;
    assert.equal(read.messages.length, count);
    for (const m of read.messages) {
      assert.equal(m.kind, "instruction");
      assert.ok(!("sender" in m), "an agent never gets an instruction's sender");
    }
    assert.doesNotMatch(JSON.stringify(read), /Muse|Aster/);
    assert.equal(read.stats.unread_instructions, count);
  }
  const gptAll = (await tool(GPT, "read_messages", { mark_read: true })).structuredContent;
  assert.deepEqual(gptAll.messages.map((m) => m.kind), ["instruction", "instruction", "message"]);
  assert.equal(gptAll.messages[2].sender, "Drew", "a plain message keeps its sender");
  assert.equal(gptAll.stats.unread_instructions, 0);

  // Drew's own views keep the sender.
  const api = await (await call(`/api/${TOKEN}/messages?kind=instruction`)).json();
  assert.deepEqual(api.messages.map((m) => m.sender), ["Muse", "Muse", "Aster"]);

  // Agents can't send instructions, and the kind must be known.
  const tryOut = await tool(TOKEN, "send_message", { body: "x", kind: "instruction" });
  assert.equal(tryOut.isError, true);
  for (const bad of ["order", 3]) {
    const r = await call(`/api/${TOKEN}/messages`, { method: "POST", body: JSON.stringify({ body: "x", kind: bad }) });
    assert.equal(r.status, 400, JSON.stringify(bad));
  }
  assert.equal((await call(`/api/${TOKEN}/messages?kind=nope`)).status, 400);
  const badRead = await tool(TOKEN, "read_messages", { kind: "nope" });
  assert.equal(badRead.isError, true);
});

test("only inbound messages can be instructions", async () => {
  const store = new MemoryStore();
  await assert.rejects(
    store.add({ body: "x", direction: "outbound", source: "mcp", kind: "instruction" }),
    /only inbound/,
  );
  const plain = await store.add({ body: "x", direction: "inbound", source: "api" });
  assert.equal(plain.kind, "message");
});

test("the deploy reads the uploaded version id from cf's output", async () => {
  const { uploadedVersionId, describeVersions } = await import("../deploy-probe.mjs");
  const out = "│  Uploaded message-bridge (0.92 sec)\n│  Worker Version ID: C8467CA5-4aae-4e33-bfcd-4ed13b280d06\n│\n";
  assert.equal(uploadedVersionId(out), "c8467ca5-4aae-4e33-bfcd-4ed13b280d06");
  assert.equal(uploadedVersionId("Uploaded, no id here"), null);
  assert.equal(uploadedVersionId(undefined), null);
  const line = describeVersions(
    [
      { id: "a", number: 1, annotations: { "workers/message": "one" } },
      { id: "b", number: 3, annotations: {} },
    ],
    "c",
  );
  assert.match(line, /2 listed, numbers 1\.\.3/);
  assert.match(line, /^versions list.*#3 b/);
  assert.match(line, /c is not listed/);
});

test("the owner page shows a connector address that works with no sign-in", async () => {
  const { call } = harness();
  const html = await (await call(`/p/${TOKEN}`)).text();
  const addr = html.match(/id="connector" readonly value="([^"]+)"/)[1].replace(/&amp;/g, "&");
  assert.equal(addr, `${BASE}/mcp/${TOKEN}`);
  const res = await call(new URL(addr).pathname, {
    method: "POST",
    headers: { "content-type": "application/json", accept: "application/json, text/event-stream" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/list" }),
  });
  assert.equal(res.status, 200);
  assert.ok((await res.json()).result.tools.some((t) => t.name === "send_message"));
});
