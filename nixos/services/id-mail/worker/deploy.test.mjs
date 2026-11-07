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

function makeSandbox() {
  const dir = mkdtempSync(join(tmpdir(), "deploy-probe-"));
  const bin = join(dir, "bin");
  mkdirSync(bin);
  copyFileSync(join(here, "deploy.mjs"), join(dir, "deploy.mjs"));
  copyFileSync(join(here, "config.json"), join(dir, "config.json"));
  const log = join(dir, "calls.log");
  writeFileSync(log, "");
  // bws stub: logs the call, returns a stub value for every secret
  // deploy.mjs looks up (the account token plus each caller's tokenKey,
  // derived from config.json so the probe survives config edits).
  const config = JSON.parse(readFileSync(join(here, "config.json"), "utf8"));
  const keys = ["cloudflare.api-token-account-wide", ...Object.values(config.callers).map((caller) => caller.tokenKey)];
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
