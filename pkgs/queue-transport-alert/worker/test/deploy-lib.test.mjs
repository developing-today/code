import assert from "node:assert/strict";
import { test } from "node:test";

import { listItems, newestWithTag, parseArgs, parseJsonOutput, servingCheck } from "../deploy-lib.mjs";

test("parseArgs accepts the three shapes", () => {
  assert.deepEqual(parseArgs([]), { dryRun: false, deployVersion: null });
  assert.deepEqual(parseArgs(["--dry-run"]), { dryRun: true, deployVersion: null });
  assert.deepEqual(parseArgs(["--deploy-version", "abc"]), { dryRun: false, deployVersion: "abc" });
  assert.deepEqual(parseArgs(["--deploy-version=abc", "--dry-run"]), { dryRun: true, deployVersion: "abc" });
});

test("parseArgs fails closed on anything else", () => {
  for (const argv of [
    ["--dry-rnu"],
    ["deploy"],
    ["--deploy-version"],
    ["--deploy-version", "--dry-run"],
    ["--deploy-version="],
    ["--dry-run", "--dry-run"],
    ["--deploy-version", "a", "--deploy-version", "b"],
  ]) {
    assert.throws(() => parseArgs(argv), /usage:/, JSON.stringify(argv));
  }
});

test("parseJsonOutput skips progress lines before the JSON", () => {
  assert.deepEqual(parseJsonOutput('[{"id":"v1"}]'), [{ id: "v1" }]);
  assert.deepEqual(parseJsonOutput('Fetching {versions}\n{"result":{"items":[]}}'), { result: { items: [] } });
  assert.throws(() => parseJsonOutput("no json here"), /could not parse/);
});

test("listItems finds the array in every shape cf and the API use", () => {
  const items = [{ id: "a" }];
  for (const shape of [items, { result: items }, { result: { items } }, { items }, { versions: items }]) {
    assert.deepEqual(listItems(shape, "versions"), items);
  }
  assert.throws(() => listItems({ result: {} }, "versions"), /unexpected versions/);
});

test("newestWithTag picks the newest upload of a re-deployed commit", () => {
  const versions = [
    { id: "old", annotations: { "workers/tag": "git-abc" }, metadata: { created_on: "2026-11-19T01:00:00Z" } },
    { id: "new", annotations: { "workers/tag": "git-abc" }, metadata: { created_on: "2026-11-19T02:00:00Z" } },
    { id: "other", annotations: { "workers/tag": "git-def" }, metadata: { created_on: "2026-11-19T03:00:00Z" } },
  ];
  assert.equal(newestWithTag(versions, "git-abc").id, "new");
  assert.equal(newestWithTag(versions, "git-zzz"), null);
});

test("servingCheck requires the latest deployment to serve the version at 100%", () => {
  const deployments = [
    { created_on: "2026-11-19T01:00:00Z", versions: [{ version_id: "v1", percentage: 100 }] },
    { created_on: "2026-11-19T02:00:00Z", versions: [{ version_id: "v2", percentage: 100 }] },
  ];
  assert.deepEqual(servingCheck(deployments, "v2"), { ok: true });
  assert.equal(servingCheck(deployments, "v1").ok, false);
  const split = [{ created_on: "2026-11-19T03:00:00Z", versions: [{ version_id: "v2", percentage: 50 }, { version_id: "v3", percentage: 50 }] }];
  assert.match(servingCheck(split, "v2").serving, /v2@50%, v3@50%/);
});
