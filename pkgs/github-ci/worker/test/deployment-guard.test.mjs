import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";

const SCRIPT = new URL("../scripts/deployment-guard.mjs", import.meta.url);
const SHA = "5f4dcc3b5aa765d61d8327deb882cf9921f7e3e0";
const VERSION_ID = "01234567-89ab-cdef-0123-456789abcdef";
const SECRET_NAMES = [
  "GITHUB_WEBHOOK_SECRET",
  "TURSO_PLATFORM_TOKEN",
  "TURSO_QUEUE_TOKEN",
  "EVENTS_BROKER_TOKEN",
  "EVENTS_BROKER_ACCESS_CLIENT_ID",
  "EVENTS_BROKER_ACCESS_CLIENT_SECRET",
];

function candidate(overrides = {}) {
  return {
    id: VERSION_ID,
    created_on: "2026-11-22T12:00:00.000Z",
    annotations: {
      "workers/tag": `git-${SHA.slice(0, 12)}`,
      "workers/message": `github-ci candidate ${SHA}`,
    },
    bindings: SECRET_NAMES.map((name) => ({ name, type: "secret_text" })),
    ...overrides,
  };
}

function run(root, args, env = {}) {
  return spawnSync(process.execPath, [SCRIPT.pathname, ...args], {
    cwd: root,
    encoding: "utf8",
    env: { ...process.env, ...env },
  });
}

test("upload selection binds the exact full commit tag/message and outputs an ID", (t) => {
  const root = mkdtempSync(join(tmpdir(), "github-ci-guard-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const versionsPath = join(root, "versions.json");
  const outputPath = join(root, "output");
  const summaryPath = join(root, "summary");
  writeFileSync(versionsPath, JSON.stringify({ result: [candidate()] }));
  const result = run(root, ["select-upload", versionsPath, SHA], {
    GITHUB_OUTPUT: outputPath,
    GITHUB_STEP_SUMMARY: summaryPath,
  });
  assert.equal(result.status, 0, result.stderr);
  assert.match(readFileSync(outputPath, "utf8"), new RegExp(`version_id=${VERSION_ID}`));
  assert.match(readFileSync(summaryPath, "utf8"), new RegExp(SHA));
  assert.notEqual(run(root, ["select-upload", versionsPath, "a".repeat(40)]).status, 0);
});

test("version read-back requires CI provenance and every secret binding by name", (t) => {
  const root = mkdtempSync(join(tmpdir(), "github-ci-guard-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const versionPath = join(root, "version.json");
  const outputPath = join(root, "output");
  const summaryPath = join(root, "summary");
  writeFileSync(versionPath, JSON.stringify({ result: candidate() }));
  const result = run(root, ["verify-version", versionPath, VERSION_ID, SHA], {
    GITHUB_OUTPUT: outputPath,
    GITHUB_STEP_SUMMARY: summaryPath,
  });
  assert.equal(result.status, 0, result.stderr);
  assert.match(readFileSync(outputPath, "utf8"), new RegExp(`source_sha=${SHA}`));

  const droppedSecret = candidate({ bindings: candidate().bindings.slice(1) });
  writeFileSync(versionPath, JSON.stringify({ result: droppedSecret }));
  assert.notEqual(run(root, ["verify-version", versionPath, VERSION_ID, SHA], { GITHUB_OUTPUT: outputPath }).status, 0);

  const wrongProvenance = candidate({ annotations: { "workers/tag": "manual", "workers/message": "manual" } });
  writeFileSync(versionPath, JSON.stringify({ result: wrongProvenance }));
  assert.notEqual(run(root, ["verify-version", versionPath, VERSION_ID], { GITHUB_OUTPUT: outputPath }).status, 0);
});

test("version read-back accepts Cloudflare's env map shape without needing secret values", (t) => {
  const root = mkdtempSync(join(tmpdir(), "github-ci-guard-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const versionPath = join(root, "version.json");
  const outputPath = join(root, "output");
  const summaryPath = join(root, "summary");
  const version = candidate({
    bindings: undefined,
    env: Object.fromEntries(SECRET_NAMES.map((name) => [name, { type: "secret_text" }])),
  });
  writeFileSync(versionPath, JSON.stringify({ success: true, result: version }));
  const result = run(root, ["verify-version", versionPath, VERSION_ID, SHA], {
    GITHUB_OUTPUT: outputPath,
    GITHUB_STEP_SUMMARY: summaryPath,
  });
  assert.equal(result.status, 0, result.stderr);
  assert.doesNotMatch(readFileSync(summaryPath, "utf8"), /secret_text/);
});

test("promotion requires deployability and latest 100 percent read-back", (t) => {
  const root = mkdtempSync(join(tmpdir(), "github-ci-guard-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const apiPath = join(root, "api.json");
  const outputPath = join(root, "output");
  const summaryPath = join(root, "summary");

  writeFileSync(apiPath, JSON.stringify({ result: { items: [{ id: VERSION_ID }] } }));
  assert.equal(run(root, ["verify-deployable", apiPath, VERSION_ID]).status, 0);
  assert.notEqual(run(root, ["verify-deployable", apiPath, "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"]).status, 0);

  writeFileSync(apiPath, JSON.stringify({ result: [{
    created_on: "2026-11-22T12:00:00.000Z",
    versions: [{ version_id: VERSION_ID, percentage: 100 }],
  }] }));
  const ok = run(root, ["verify-deployment", apiPath, VERSION_ID, SHA], {
    GITHUB_OUTPUT: outputPath,
    GITHUB_STEP_SUMMARY: summaryPath,
  });
  assert.equal(ok.status, 0, ok.stderr);

  writeFileSync(apiPath, JSON.stringify({ result: [
    {
      created_on: "2026-11-22T12:01:00.000Z",
      versions: [{ version_id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", percentage: 100 }],
    },
    {
      created_on: "2026-11-22T12:00:00.000Z",
      versions: [{ version_id: VERSION_ID, percentage: 100 }],
    },
  ] }));
  assert.notEqual(run(root, ["verify-deployment", apiPath, VERSION_ID, SHA], { GITHUB_OUTPUT: outputPath }).status, 0);
});

test("CLI rejects unknown arguments before reading inputs", () => {
  const result = run(process.cwd(), ["version", "--dry-rnu", "/does/not/exist"]);
  assert.equal(result.status, 2);
  assert.match(result.stderr, /Unknown option/);
});
