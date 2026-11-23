// Offline validation of the pkarr release workflow's inputs and receipts.
// Commands: input; upload-id <cf-result>; receipt <deployments> <version>;
// authorize <receipt> <run-attempt> <comparison> <version> <deployments>;
// verify <deployments> <receipt>; unchanged <before> <after>.
// All evidence inputs are local files or the workflow's environment.
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

export const REPO = "developing-today/code";
export const WORKFLOW = ".github/workflows/pkarr-worker-deploy.yml";
const WORKER = "pkarr";
const uuid = /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/;
const sha = /^[a-f0-9]{40}$/;
const positive = /^[1-9][0-9]{0,19}$/;
const fail = (message) => { throw new Error(message); };
const matches = (regex, value) => typeof value === "string" && regex.test(value);
const exactKeys = (value, expected) => value && typeof value === "object" &&
  !Array.isArray(value) && JSON.stringify(Object.keys(value).sort()) ===
  JSON.stringify([...expected].sort());

export function dispatchInput({ operation, version, run, attempt, source }) {
  if (operation === "upload" && !version && !run && !attempt && !source) return;
  if (operation === "promote" && matches(uuid, version) &&
      matches(positive, run) && matches(positive, attempt) && matches(sha, source)) return;
  fail("invalid operation or upload evidence inputs");
}

export function uploadVersion(output) {
  // Verified shape from cf@1.0.0-beta.32's structured versions-upload emitter.
  if (output?.type !== "version-upload" || output.version !== 1 ||
      output.worker_name !== WORKER || output.worker_name_overridden !== false ||
      !matches(uuid, output.version_id)) fail("invalid pkarr upload response");
  return output.version_id;
}

export function serving(response) {
  const deployments = response?.result?.deployments;
  if (response?.success !== true || !Array.isArray(deployments) ||
      deployments.length === 0 || deployments.some((d) =>
        !matches(uuid, d?.id) || typeof d.created_on !== "string" ||
        !Number.isFinite(Date.parse(d.created_on)))) fail("unverified pkarr deployments");
  const ordered = [...deployments].sort((a, b) =>
    Date.parse(b.created_on) - Date.parse(a.created_on));
  const current = ordered[0];
  if (ordered[1] && Date.parse(current.created_on) === Date.parse(ordered[1].created_on)) {
    fail("ambiguous latest pkarr deployment");
  }
  const versions = current.versions;
  if (!Array.isArray(versions) || versions.length === 0 ||
      versions.some((v) => !matches(uuid, v?.version_id) ||
        typeof v.percentage !== "number" || !Number.isFinite(v.percentage) ||
        v.percentage < 0 || v.percentage > 100) ||
      new Set(versions.map((v) => v.version_id)).size !== versions.length ||
      Math.abs(versions.reduce((sum, v) => sum + v.percentage, 0) - 100) > 1e-9) {
    fail("unverified pkarr traffic split");
  }
  return {
    id: current.id,
    versions: versions.map(({ version_id, percentage }) => ({ version_id, percentage }))
      .sort((a, b) => a.version_id.localeCompare(b.version_id)),
  };
}

export function message(source, run, attempt) {
  return `pkarr ${source} run ${run} attempt ${attempt}`;
}

export function receipt({ account, version, source, run, attempt, baseline }) {
  if (!matches(/^[a-f0-9]{32}$/, account) || !matches(uuid, version) ||
      !matches(sha, source) || !matches(positive, run) || !matches(positive, attempt)) {
    fail("invalid receipt identity");
  }
  return {
    schema: 1, repository: REPO, workflow: WORKFLOW, worker: WORKER,
    account_sha256: createHash("sha256").update(account).digest("hex"),
    version, source, run, attempt, baseline,
  };
}

export function confirmVersion(response, { version, source, run, attempt }) {
  const v = response?.result;
  if (response?.success !== true || v?.id !== version ||
      v.annotations?.["workers/tag"] !== source ||
      v.annotations?.["workers/message"] !== message(source, run, attempt)) {
    fail("pkarr version does not match exact upload provenance");
  }
}

export function authorize({ saved, upload, comparison, versionResult, current, expected, main }) {
  const expectedReceipt = receipt({ ...expected, baseline: serving(current) });
  if (!exactKeys(saved, Object.keys(expectedReceipt)) || Object.entries(expectedReceipt)
      .some(([key, value]) => JSON.stringify(saved[key]) !== JSON.stringify(value))) {
    fail("pkarr upload receipt or serving baseline no longer matches");
  }
  if (String(upload?.id) !== expected.run || String(upload?.run_attempt) !== expected.attempt ||
      ![WORKFLOW, `${WORKFLOW}@refs/heads/main`].includes(upload?.path) ||
      upload.event !== "workflow_dispatch" || upload.head_branch !== "main" ||
      upload.head_sha !== expected.source || upload.status !== "completed" ||
      upload.conclusion !== "success" || upload.repository?.full_name !== REPO ||
      upload.head_repository?.full_name !== REPO) fail("untrusted pkarr upload run attempt");
  if (!matches(sha, main) || comparison?.base_commit?.sha !== expected.source ||
      comparison?.merge_base_commit?.sha !== expected.source ||
      !["ahead", "identical"].includes(comparison?.status) ||
      (comparison.status === "identical") !== (main === expected.source)) {
    fail("pkarr upload source is not a verified ancestor of main");
  }
  confirmVersion(versionResult, expected);
}

export function verifyPromotion(response, version, baselineId) {
  const observed = serving(response);
  if (observed.id === baselineId || observed.versions.length !== 1 ||
      observed.versions[0].version_id !== version || observed.versions[0].percentage !== 100) {
    fail("pkarr promotion is not serving the exact candidate at 100%");
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const [operation, ...files] = process.argv.slice(2);
    const read = (i) => JSON.parse(readFileSync(files[i], "utf8"));
    const env = process.env;
    const expected = {
      account: env.CLOUDFLARE_ACCOUNT_ID, version: env.VERSION_ID,
      source: env.UPLOAD_SOURCE_SHA, run: env.UPLOAD_RUN_ID, attempt: env.UPLOAD_RUN_ATTEMPT,
    };
    if (operation === "input" && files.length === 0) {
      dispatchInput({ operation: env.OPERATION, ...expected });
    } else if (operation === "upload-id" && files.length === 1) {
      console.log(uploadVersion(read(0)));
    } else if (operation === "receipt" && files.length === 2) {
      confirmVersion(read(1), expected);
      console.log(JSON.stringify(receipt({ ...expected, baseline: serving(read(0)) })));
    } else if (operation === "authorize" && files.length === 5) {
      authorize({ saved: read(0), upload: read(1), comparison: read(2), versionResult: read(3),
        current: read(4), expected, main: env.GITHUB_SHA });
    } else if (operation === "verify" && files.length === 2) {
      verifyPromotion(read(0), expected.version, read(1)?.baseline?.id);
    } else if (operation === "unchanged" && files.length === 2) {
      if (JSON.stringify(serving(read(0))) !== JSON.stringify(serving(read(1)))) {
        fail("pkarr serving deployment changed during upload");
      }
    } else fail("invalid pkarr publish validation invocation");
  } catch {
    // Provider payloads and credentials must never flow into Actions logs.
    console.error("pkarr publish evidence validation failed; inspect the named step before retrying");
    process.exitCode = 1;
  }
}
