import assert from "node:assert/strict";
import test from "node:test";
import { assertRouteHealth, assertVersionConfigurationUnchanged, validateManifest } from "./promote.mjs";
import { verifyUploadRun } from "./verify-upload-run.mjs";

const manifest = {
  schema: 1,
  repository: "developing-today/code",
  workflow: ".github/workflows/id-mail-worker-publish.yml",
  event: "workflow_dispatch",
  ref: "refs/heads/main",
  sha: "a".repeat(40),
  runId: "1234",
  runAttempt: "2",
  worker: "mail",
  tag: `mail-${"a".repeat(12)}-run-1234-attempt-2`,
  versionId: "version-new",
  baselineDeploymentId: "deployment-old",
  baselineVersionId: "version-old",
};

test("promotion provenance is pinned to the selected upload attempt", () => {
  validateManifest(manifest, {
    expectedRunId: "1234",
    expectedAttempt: "2",
    repository: "developing-today/code",
    worker: "mail",
    expectedSha: "a".repeat(40),
  });
  assert.throws(() => validateManifest({ ...manifest, runAttempt: "1" }, {
    expectedRunId: "1234", expectedAttempt: "2", repository: "developing-today/code", worker: "mail", expectedSha: "a".repeat(40),
  }), /provenance/);
});

test("only a successful main-branch manual publisher attempt is accepted", () => {
  const run = {
    id: 1234,
    run_attempt: 2,
    status: "completed",
    conclusion: "success",
    event: "workflow_dispatch",
    head_branch: "main",
    head_sha: "a".repeat(40),
    path: ".github/workflows/id-mail-worker-publish.yml",
    repository: { full_name: "developing-today/code" },
  };
  const verified = verifyUploadRun(run, { runId: "1234", attempt: "2", repository: "developing-today/code" });
  assert.equal(verified.sha, run.head_sha);
  assert.throws(() => verifyUploadRun({ ...run, event: "pull_request" }, {
    runId: "1234", attempt: "2", repository: "developing-today/code",
  }), /successful manual upload/);
});

test("CALLERS is inherited by name and type while secret contents are never compared", () => {
  const bindings = [
    { name: "CALLERS", type: "secret_text" },
    { name: "EMAIL", type: "send_email", allowed_sender_addresses: ["id@example.test"] },
    { name: "LIMITER", type: "durable_object", class_name: "Limiter" },
    { name: "MAIL_CONFIG", type: "plain_text", text: "same config" },
  ];
  const before = { bindings, compatibility_date: "2026-10-01", compatibility_flags: [] };
  const after = {
    ...before,
    bindings: bindings.map((binding) => binding.name === "CALLERS" ? { ...binding } : { ...binding }),
  };
  assertVersionConfigurationUnchanged(before, after);
  assert.throws(() => assertVersionConfigurationUnchanged(before, {
    ...after,
    bindings: after.bindings.map((binding) => binding.name === "CALLERS" ? { name: "CALLERS", type: "plain_text" } : binding),
  }), /CALLERS secret binding/);
  assert.throws(() => assertVersionConfigurationUnchanged(before, {
    ...after,
    observability: { enabled: true },
  }), /settings/);
});

test("safe health probe is an unauthenticated POST and accepts only expected 401 JSON", async () => {
  const calls = [];
  await assertRouteHealth(["mail.example.test"], async (url, options) => {
    calls.push({ url, options });
    return new Response(JSON.stringify({ error: "unauthorized" }), {
      status: 401,
      headers: { "content-type": "application/json" },
    });
  });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, "https://mail.example.test/");
  assert.equal(calls[0].options.method, "POST");
  assert.equal(calls[0].options.body, undefined);
  assert.equal(calls[0].options.headers, undefined);
});
