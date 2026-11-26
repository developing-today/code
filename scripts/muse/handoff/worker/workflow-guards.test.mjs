import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../../../");
const uploadPath = resolve(root, ".github/workflows/muse-handoff-upload.yml");
const promotePath = resolve(root, ".github/workflows/muse-handoff-promote.yml");

test("upload workflow is manual, main-only, environment-scoped, and does not promote production", async () => {
  const source = await readFile(uploadPath, "utf8");
  assert.match(source, /workflow_dispatch:/);
  assert.doesNotMatch(source, /pull_request|pull_request_target|\n\s+push:/);
  assert.match(source, /if: github\.ref == 'refs\/heads\/main'/);
  assert.match(source, /environment: muse-handoff-staging/);
  assert.match(source, /environment: muse-handoff-production/);
  assert.match(source, /MUSE_HANDOFF_STAGING_CF_API_TOKEN/);
  assert.match(source, /MUSE_HANDOFF_PRODUCTION_CF_API_TOKEN/);
  assert.match(source, /vars\.MUSE_HANDOFF_STAGING_WORKERS_DEV_HOST/);
  assert.match(source, /permissions:\n  contents: read/);
  assert.match(source, /workers versions create/);
  assert.equal((source.match(/workers deployments create/g) ?? []).length, 1);
  const productionUpload = source.split("  upload-production-version:")[1];
  assert.doesNotMatch(productionUpload, /workers deployments create/);
  assert.doesNotMatch(source, /wrangler\s+(deploy|versions upload)/);
  assert.doesNotMatch(source, /"[^"\n]*`[^"\n]*"/);
  assert.match(source, /stage-smoke\.mjs verify/);
});

test("promotion requires upload provenance and stage proof, then checks active production traffic", async () => {
  const source = await readFile(promotePath, "utf8");
  assert.match(source, /workflow_dispatch:/);
  assert.doesNotMatch(source, /pull_request|pull_request_target|\n\s+push:/);
  assert.match(source, /if: github\.ref == 'refs\/heads\/main'/);
  assert.match(source, /environment: muse-handoff-production/);
  assert.match(source, /actual\.path !== "\.github\/workflows\/muse-handoff-upload\.yml"/);
  assert.match(source, /actual\.conclusion !== "success"/);
  assert.match(source, /verify-release/);
  assert.match(source, /assert-same-active/);
  assert.match(source, /assert-active/);
  assert.match(source, /workers deployments create/);
  assert.doesNotMatch(source, /staged_parity_verified|version_id:\s*$/);
  assert.doesNotMatch(source, /wrangler\s+(deploy|versions upload)/);
  assert.doesNotMatch(source, /"[^"\n]*`[^"\n]*"/);
});
