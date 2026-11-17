// Probe for deploy.mjs argument handling (issue #3281).
//
// Runs deploy.mjs in a temp dir with stubbed `bws` / `wrangler` binaries on
// PATH and a stubbed global fetch (via --import), and pins the three cases:
//   --dry-run   -> validation only, exits 0, nothing deployed
//   --dry-rnu   -> usage error, non-zero exit, ZERO external calls logged
//   (no args)   -> the real deploy sequence runs against the stubs
// Run with: node --test deploy.test.mjs
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const run = promisify(execFile);
const here = dirname(fileURLToPath(import.meta.url));

function makeSandbox(mutateConfig) {
  const dir = mkdtempSync(join(tmpdir(), "deploy-probe-"));
  const bin = join(dir, "bin");
  mkdirSync(bin);
  copyFileSync(join(here, "deploy.mjs"), join(dir, "deploy.mjs"));
  // The sandbox config defaults to the committed one; a test may replace
  // it wholesale (a raw string) or mutate the parsed object (#6947).
  let configText = readFileSync(join(here, "config.json"), "utf8");
  if (typeof mutateConfig === "string") {
    configText = mutateConfig;
  } else if (typeof mutateConfig === "function") {
    const mutated = JSON.parse(configText);
    mutateConfig(mutated);
    configText = JSON.stringify(mutated, null, 2);
  }
  writeFileSync(join(dir, "config.json"), configText);
  const log = join(dir, "calls.log");
  writeFileSync(log, "");
  // bws stub: logs the call, returns a stub value for every secret
  // deploy.mjs looks up (the account token plus each caller's tokenKey,
  // derived from config.json so the probe survives config edits).
  // Malformed configs never reach bws, so unparseable shapes simply
  // contribute no caller keys.
  let keys = ["cloudflare.api-token-account-wide"];
  try {
    const config = JSON.parse(configText);
    keys = [
      ...keys,
      ...Object.values(config.callers ?? {})
        .filter((caller) => caller && typeof caller.tokenKey === "string")
        .map((caller) => caller.tokenKey),
    ];
  } catch {
    /* the invalid-config cases exit in validation, before any bws call */
  }
  const secretsJson = JSON.stringify(keys.map((key) => ({ key, value: `stub-${key}` })));
  writeFileSync(join(bin, "bws"), `#!/bin/sh\necho "bws $@" >> "${log}"\nprintf '%s' '${secretsJson}'\n`);
  writeFileSync(join(bin, "wrangler"), `#!/bin/sh\necho "wrangler $@" >> "${log}"\nexit 0\n`);
  chmodSync(join(bin, "bws"), 0o755);
  chmodSync(join(bin, "wrangler"), 0o755);
  // fetch stub: every retired Worker reports 404 (already gone), so the
  // retire loop logs and moves on without a wrangler delete.
  const fetchStub = join(dir, "fetch-stub.mjs");
  writeFileSync(fetchStub, `globalThis.fetch = async () => ({ status: 404, ok: false });\n`);
  return { dir, bin, log, fetchStub };
}

async function runDeploy(sb, args) {
  try {
    const { stdout, stderr } = await run("node", ["--import", sb.fetchStub, join(sb.dir, "deploy.mjs"), ...args], {
      env: {
        ...process.env,
        PATH: `${sb.bin}:${process.env.PATH}`,
        CLOUDFLARE_ACCOUNT_ID: "stub-account",
      },
    });
    return { code: 0, stdout, stderr };
  } catch (error) {
    return { code: error.code, stdout: error.stdout ?? "", stderr: error.stderr ?? "" };
  }
}

test("--dry-run validates only and deploys nothing", async () => {
  const sb = makeSandbox();
  const result = await runDeploy(sb, ["--dry-run"]);
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /nothing deployed/);
  const calls = readFileSync(sb.log, "utf8");
  assert.match(calls, /bws secret list/);
  assert.match(calls, /wrangler deploy --dry-run/);
  assert.doesNotMatch(calls, /wrangler deploy --config/);
  assert.doesNotMatch(calls, /secret put/);
});

test("--dry-rnu (typo) aborts with a usage error before any external call", async () => {
  const sb = makeSandbox();
  const result = await runDeploy(sb, ["--dry-rnu"]);
  assert.notEqual(result.code, 0);
  assert.match(result.stderr, /usage: deploy\.mjs \[--dry-run\]/);
  assert.equal(readFileSync(sb.log, "utf8"), "", "no bws/wrangler call may be logged");
});

test("--bogus-flag aborts with a usage error before any external call", async () => {
  const sb = makeSandbox();
  const result = await runDeploy(sb, ["--bogus-flag"]);
  assert.notEqual(result.code, 0);
  assert.match(result.stderr, /unrecognized argument\(s\): --bogus-flag/);
  assert.equal(readFileSync(sb.log, "utf8"), "", "no bws/wrangler call may be logged");
});

test("null and non-object config entries are validation errors, not TypeErrors (#6947)", async () => {
  const cases = [
    ["null", /config must be a JSON object/],
    [
      (c) => {
        c.templates = [null];
      },
      /templates\[0\] must be an object/,
    ],
    [
      (c) => {
        c.callers.amd = null;
      },
      /callers\.amd must be an object/,
    ],
    [
      (c) => {
        c.callers.amd.transports.email = null;
      },
      /callers\.amd\.transports\.email must be an object/,
    ],
    [
      (c) => {
        c.transports = "email";
      },
      /transports must be a non-empty list/,
    ],
    [
      (c) => {
        c.transports = 5;
      },
      /transports must be a non-empty list/,
    ],
    [
      (c) => {
        c.templates = 5;
      },
      /templates must be a list/,
    ],
  ];
  for (const [mutate, pattern] of cases) {
    const sb = makeSandbox(mutate);
    const result = await runDeploy(sb, ["--dry-run"]);
    assert.notEqual(result.code, 0);
    assert.match(result.stderr, /config\.json is invalid:/);
    assert.match(result.stderr, pattern);
    assert.doesNotMatch(result.stderr, /TypeError/);
    assert.equal(readFileSync(sb.log, "utf8"), "", "no bws/wrangler call may be logged");
  }
});

test("a config without retire validates and deploys fully against the stubs (#6947)", async () => {
  const sb = makeSandbox((c) => {
    delete c.retire;
  });
  const dry = await runDeploy(sb, ["--dry-run"]);
  assert.equal(dry.code, 0, dry.stderr);
  assert.match(dry.stdout, /nothing deployed/);
  const result = await runDeploy(sb, []);
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /deployed mail with callers/);
  const calls = readFileSync(sb.log, "utf8");
  assert.match(calls, /wrangler deploy --config/);
  assert.match(calls, /secret put/);
});

test("no args runs the real deploy sequence against the stubs", async () => {
  const sb = makeSandbox();
  const result = await runDeploy(sb, []);
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /deployed mail with callers/);
  const calls = readFileSync(sb.log, "utf8");
  assert.match(calls, /bws secret list/);
  assert.match(calls, /wrangler deploy --dry-run/);
  assert.match(calls, /wrangler deploy --config/);
  assert.match(calls, /wrangler secret put CALLERS/);
});
