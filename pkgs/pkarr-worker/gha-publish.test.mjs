import assert from "node:assert/strict";
import test from "node:test";
import {
  authorize, dispatchInput, message, receipt, REPO, serving, uploadVersion,
  verifyPromotion, WORKFLOW,
} from "./gha-publish.mjs";

const version = "1".repeat(8) + "-1111-1111-1111-" + "1".repeat(12);
const other = "2".repeat(8) + "-2222-2222-2222-" + "2".repeat(12);
const prior = "3".repeat(8) + "-3333-3333-3333-" + "3".repeat(12);
const account = "a".repeat(32);
const source = "b".repeat(40);
const main = "c".repeat(40);
const run = "12345";
const attempt = "2";
const expected = { account, version, source, run, attempt };
const deployment = (id, version_id, created_on) => ({ id, created_on, versions: [{ version_id, percentage: 100 }] });
const current = { success: true, result: { deployments: [deployment(prior, other, "2026-11-23T06:00:00Z")] } };
const validRun = {
  id: Number(run), run_attempt: Number(attempt), path: WORKFLOW, event: "workflow_dispatch",
  head_branch: "main", head_sha: source, status: "completed", conclusion: "success",
  repository: { full_name: REPO }, head_repository: { full_name: REPO },
};
const versionResult = {
  success: true,
  result: { id: version, annotations: { "workers/tag": source, "workers/message": message(source, run, attempt) } },
};
const evidence = () => ({
  current: structuredClone(current), expected, main, upload: structuredClone(validRun),
  saved: receipt({ ...expected, baseline: serving(current) }),
  versionResult: structuredClone(versionResult),
  comparison: { status: "ahead", base_commit: { sha: source }, merge_base_commit: { sha: source } },
});

test("exact dispatch input and pinned receipt format, never tag-list guesswork", () => {
  dispatchInput({ operation: "upload" });
  dispatchInput({ operation: "promote", ...expected });
  assert.throws(() => dispatchInput({ operation: "upload", version }));
  assert.throws(() => dispatchInput({ operation: "promote", ...expected, attempt: "0" }));
  assert.throws(() => dispatchInput({ operation: "promote", ...expected, source: source.slice(0, 12) }));
  const exact = { type: "version-upload", version: 1, worker_name: "pkarr", worker_name_overridden: false, version_id: version };
  assert.equal(uploadVersion(exact), version);
  for (const drift of [{ version: 2 }, { worker_name: "other" }, { worker_name_overridden: true }, { version_id: "newest-by-tag" }]) {
    assert.throws(() => uploadVersion({ ...exact, ...drift }), /invalid pkarr upload response/);
  }
});

test("promote only the exact GitHub attempt, account, worker, full reachable SHA and candidate", () => {
  assert.doesNotThrow(() => authorize(evidence()));
  for (const corrupt of [
    (x) => { x.saved.worker = "not-pkarr"; },
    (x) => { x.saved.account_sha256 = "f".repeat(64); },
    (x) => { x.saved.run = "11111"; },
    (x) => { x.saved.version = other; },
    (x) => { x.upload.run_attempt++; },
    (x) => { x.upload.head_repository.full_name = "fork/code"; },
    (x) => { x.upload.conclusion = "failure"; },
    (x) => { x.upload.head_sha = main; },
    (x) => { x.comparison.status = "diverged"; },
    (x) => { x.comparison.merge_base_commit.sha = main; },
    (x) => { x.versionResult.result.id = other; },
    (x) => { x.versionResult.result.annotations["workers/tag"] = source.slice(0, 12); },
    (x) => { x.versionResult.result.annotations["workers/message"] = message(source, run, "1"); },
  ]) {
    const x = evidence();
    corrupt(x);
    assert.throws(() => authorize(x));
  }
  const x = evidence();
  assert.equal(JSON.stringify(x.saved).includes(account), false);
});

test("reject stale or ambiguous serving baseline, then verify exact new 100% serving state", () => {
  const x = evidence();
  x.current.result.deployments.push(deployment(other, other, "2026-11-23T07:00:00Z"));
  assert.throws(() => authorize(x), /serving baseline no longer matches/);
  x.current.result.deployments[1].created_on = "2026-11-23T06:00:00Z";
  assert.throws(() => authorize(x), /ambiguous latest/);
  const after = structuredClone(current);
  after.result.deployments.unshift(deployment(other, version, "2026-11-23T07:00:00Z"));
  assert.doesNotThrow(() => verifyPromotion(after, version, prior));
  assert.throws(() => verifyPromotion(current, version, prior), /not serving/);
  after.result.deployments[0].versions = [{ version_id: version, percentage: 99 }, { version_id: other, percentage: 1 }];
  assert.throws(() => verifyPromotion(after, version, prior), /not serving/);
});
