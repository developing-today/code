// Probe for deploy.mjs argument handling and upload-only behavior.
// Runs deploy.mjs in a temp dir with a stubbed project-local `cf` binary.
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
  const cfBin = join(dir, "node_modules", ".bin");
  mkdirSync(cfBin, { recursive: true });
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
  writeFileSync(join(cfBin, "cf"), `#!/bin/sh\necho "cf $@" >> "${log}"\nexit 0\n`);
  chmodSync(join(cfBin, "cf"), 0o755);
  return { dir, cfBin, log };
}

async function runDeploy(sb, args, { token = true } = {}) {
  try {
    const env = {
      PATH: process.env.PATH,
      CLOUDFLARE_ACCOUNT_ID: "stub-account",
      GITHUB_SHA: "0123456789abcdef0123456789abcdef01234567",
    };
    if (token && !args.includes("--dry-run")) env.CLOUDFLARE_API_TOKEN = "stub-token";
    const { stdout, stderr } = await run("node", [join(sb.dir, "deploy.mjs"), ...args], { env });
    return { code: 0, stdout, stderr };
  } catch (error) {
    return { code: error.code, stdout: error.stdout ?? "", stderr: error.stderr ?? "" };
  }
}

test("--dry-run validates and builds without uploading", async () => {
  const sb = makeSandbox();
  const result = await runDeploy(sb, ["--dry-run"]);
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /nothing uploaded/);
  const calls = readFileSync(sb.log, "utf8");
  assert.match(calls, /cf workers versions create --worker mail --tag git-0123456789ab/);
  assert.match(calls, /--dry-run/);
  assert.doesNotMatch(calls, /deployments create|delete|wrangler|secret put|bws/);
});

test("--dry-rnu (typo) aborts with a usage error before any external call", async () => {
  const sb = makeSandbox();
  const result = await runDeploy(sb, ["--dry-rnu"]);
  assert.notEqual(result.code, 0);
  assert.match(result.stderr, /usage: deploy\.mjs \[--dry-run\]/);
  assert.equal(readFileSync(sb.log, "utf8"), "", "no cf call may be logged");
});

test("--bogus-flag aborts with a usage error before any external call", async () => {
  const sb = makeSandbox();
  const result = await runDeploy(sb, ["--bogus-flag"]);
  assert.notEqual(result.code, 0);
  assert.match(result.stderr, /unrecognized argument\(s\): --bogus-flag/);
  assert.equal(readFileSync(sb.log, "utf8"), "", "no cf call may be logged");
});

test("a stray positional aborts with a usage error before any external call (#7855)", async () => {
  const sb = makeSandbox();
  const result = await runDeploy(sb, ["extra"]);
  assert.equal(result.code, 2, result.stderr);
  assert.match(result.stderr, /unrecognized argument\(s\): extra/);
  assert.match(result.stderr, /usage: deploy\.mjs \[--dry-run\]/);
  assert.equal(readFileSync(sb.log, "utf8"), "", "no cf call may be logged");
});

test("duplicate --dry-run is named as a duplicate, not unrecognized (#7855)", async () => {
  const sb = makeSandbox();
  const result = await runDeploy(sb, ["--dry-run", "--dry-run"]);
  assert.equal(result.code, 2, result.stderr);
  assert.match(result.stderr, /duplicate argument\(s\): --dry-run/);
  assert.doesNotMatch(result.stderr, /unrecognized argument/);
  assert.match(result.stderr, /usage: deploy\.mjs \[--dry-run\]/);
  assert.equal(readFileSync(sb.log, "utf8"), "", "no cf call may be logged");
});

test("--dry-run with a bogus flag aborts with a usage error before any external call (#7855)", async () => {
  const sb = makeSandbox();
  const result = await runDeploy(sb, ["--dry-run", "--bogus-flag"]);
  assert.equal(result.code, 2, result.stderr);
  assert.match(result.stderr, /unrecognized argument\(s\): --bogus-flag/);
  assert.match(result.stderr, /usage: deploy\.mjs \[--dry-run\]/);
  assert.equal(readFileSync(sb.log, "utf8"), "", "no cf call may be logged");
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
    assert.equal(readFileSync(sb.log, "utf8"), "", "no cf call may be logged");
  }
});

test("a config without retire validates and uploads a version (#6947)", async () => {
  const sb = makeSandbox((c) => {
    delete c.retire;
  });
  const dry = await runDeploy(sb, ["--dry-run"]);
  assert.equal(dry.code, 0, dry.stderr);
  assert.match(dry.stdout, /nothing uploaded/);
  const result = await runDeploy(sb, []);
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /uploaded mail version/);
  const calls = readFileSync(sb.log, "utf8");
  assert.match(calls, /cf workers versions create --worker mail --tag git-0123456789ab/);
  assert.doesNotMatch(calls, /deployments create|delete|wrangler|secret put|bws/);
});

test("upload requires an API token before calling cf", async () => {
  const sb = makeSandbox();
  const result = await runDeploy(sb, [], { token: false });
  assert.notEqual(result.code, 0);
  assert.match(result.stderr, /CLOUDFLARE_API_TOKEN is not set/);
  assert.equal(readFileSync(sb.log, "utf8"), "");
});

test("no args uploads a version only against the stub", async () => {
  const sb = makeSandbox();
  const result = await runDeploy(sb, []);
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /uploaded mail version/);
  const calls = readFileSync(sb.log, "utf8");
  assert.equal(calls.split("\n").filter(Boolean).length, 1);
  assert.match(calls, /^cf workers versions create --worker mail --tag git-0123456789ab/);
  assert.doesNotMatch(calls, /deployments create|delete|wrangler|secret put|bws/);
});
