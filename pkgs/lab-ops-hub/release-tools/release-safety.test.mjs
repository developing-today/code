import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import {
  assertCurrentBaseline,
  assertTrustedUploadReceipt,
  assertWorkerVersion,
  latestServingVersionId,
  makeUploadReceipt,
  readExpectedWorkerConfig,
  versionIdFromUpload,
} from "./release-safety.mjs";

const idA = "51a5a667-13c2-46e5-9aad-b9e756763b62";
const idB = "fb7f5708-0cc5-4bc4-a501-6e9be05de249";
const sha = "a".repeat(40);
const workerConfig = {
  name: "lab-ops-hub",
  migrations: [{ tag: "v1" }, { tag: "v2" }],
  containers: [{
    class_name: "CrawlerContainerV2",
    image: "registry.cloudflare.com/234b866b99ce2b142d7963d8d139cbfa/lab-ops-hub-crawlercontainer:v5",
  }],
};
const expectedConfig = readExpectedWorkerConfig(JSON.stringify(workerConfig));

function deployment(versionId, percentage = 100) {
  return { versions: [{ version_id: versionId, percentage }] };
}

function version(id, { image = expectedConfig.containers[0].image, tag } = {}) {
  return {
    result: {
      id,
      migration_tag: "v2",
      ...(tag ? { annotations: { "workers/tag": tag } } : {}),
      containers: [{ class_name: "CrawlerContainerV2", image }],
    },
    success: true,
  };
}

test("uses the first documented active deployment and requires exactly one 100% version", () => {
  const response = {
    result: {
      deployments: [deployment(idA), deployment(idB, 50)],
    },
    success: true,
  };
  assert.equal(latestServingVersionId(response), idA);
  assert.equal(assertCurrentBaseline(response, idA), idA);
  assert.throws(() => assertCurrentBaseline(response, idB), /serving baseline changed/);
  assert.throws(() => latestServingVersionId({ result: { deployments: [] } }), /no current Worker deployment/);
  assert.throws(() => latestServingVersionId({ result: { deployments: [deployment(idA, 99)] } }), /exactly one version at 100%/);
  assert.throws(() => latestServingVersionId({ result: { deployments: [{ versions: [
    { version_id: idA, percentage: 50 }, { version_id: idB, percentage: 50 },
  ] }] } }), /single-version deployment/);
});

test("requires exact ID, active v2 migration, container class, image pin, and upload tag", () => {
  const candidateTag = `git-${sha.slice(0, 12)}`;
  assert.equal(assertWorkerVersion(version(idA, { tag: candidateTag }), {
    versionId: idA,
    versionTag: candidateTag,
    expectedConfig,
  }).id, idA);
  assert.throws(() => assertWorkerVersion(version(idB), { versionId: idA, expectedConfig }), /does not match/);
  assert.throws(() => assertWorkerVersion({ result: { ...version(idA).result, migration_tag: "v1" } }, {
    versionId: idA,
    expectedConfig,
  }), /expected active tag v2/);
  assert.throws(() => assertWorkerVersion(version(idA, { image: "registry.cloudflare.com/account/other:v5" }), {
    versionId: idA,
    expectedConfig,
  }), /does not match the pinned image/);
  assert.throws(() => assertWorkerVersion(version(idA), {
    versionId: idA,
    versionTag: "git-wrongtag",
    expectedConfig,
  }), /tag metadata does not match/);
});

test("fails closed on the Cloudflare-documented Worker Version shape without an image URI", () => {
  const documentedShape = version(idA).result;
  delete documentedShape.containers[0].image;
  assert.throws(() => assertWorkerVersion({ result: documentedShape }, {
    versionId: idA,
    expectedConfig,
  }), /exposes the container class but not its image URI.*Failing closed/);
});

test("only parses exact UUIDs from cf upload results", () => {
  assert.equal(versionIdFromUpload({ result: { id: idA }, success: true }), idA);
  assert.throws(() => versionIdFromUpload({ result: { id: "51a5a667" }, success: true }), /exact Worker Version UUID/);
});

test("promotion JSON is generated only from a receipt version and CLI works from the repository root", () => {
  const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
  const script = relative(repoRoot, fileURLToPath(new URL("./release-safety.mjs", import.meta.url)));
  const tempDir = mkdtempSync(join(tmpdir(), "loh-release-safety-"));
  const receiptPath = join(tempDir, "receipt.json");
  const versionsPath = join(tempDir, "versions.json");
  try {
    writeFileSync(receiptPath, JSON.stringify({ version_id: idB }));
    const result = spawnSync(process.execPath, [script, "deployment-versions", receiptPath, versionsPath], {
      cwd: repoRoot,
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(JSON.parse(readFileSync(versionsPath, "utf8")), [{ version_id: idB, percentage: 100 }]);
  } finally {
    rmSync(tempDir, { recursive: true, force: true });
  }
});

test("receipt is tied to the exact successful trusted upload run and baseline", () => {
  const tag = `git-${sha.slice(0, 12)}`;
  const receipt = makeUploadReceipt({
    uploadResponse: { result: { id: idB }, success: true },
    versionResponse: version(idB, { tag }),
    baselineVersionId: idA,
    context: {
      sourceSha: sha,
      runId: "987654321",
      runAttempt: "2",
      repository: "developing-today/code",
      ref: "refs/heads/main",
      event: "workflow_dispatch",
    },
  });
  const run = {
    id: 987654321,
    run_attempt: 2,
    status: "completed",
    conclusion: "success",
    event: "workflow_dispatch",
    head_branch: "main",
    head_sha: sha,
    path: ".github/workflows/lab-ops-hub-worker-release.yml",
    repository: { full_name: "developing-today/code" },
  };
  assert.equal(assertTrustedUploadReceipt(receipt, run).version_id, idB);
  for (const change of [
    { run_attempt: 1 },
    { status: "in_progress" },
    { conclusion: "failure" },
    { event: "pull_request" },
    { head_branch: "feature" },
    { head_sha: "b".repeat(40) },
    { path: "other.yml" },
    { repository: { full_name: "attacker/other" } },
    { repository: { name: "code" } },
  ]) {
    assert.throws(() => assertTrustedUploadReceipt(receipt, { ...run, ...change }), /exact successful trusted main-branch upload run/);
  }
  assert.throws(() => assertTrustedUploadReceipt({ ...receipt, baseline_version_id: "not-a-uuid" }, run), /does not match/);
});
