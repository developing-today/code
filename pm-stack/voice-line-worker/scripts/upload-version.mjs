// Upload a prebuilt Worker Version without creating or changing a deployment.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { exactWorkerId, parseCfJson, parseVersionItems, releaseRepository, releaseWorker, requireExactVersion, uploadedVersionId } from "../src/deploy-contract.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const workerDir = resolve(here, "..");
if (process.env.GITHUB_REPOSITORY !== releaseRepository || process.env.GITHUB_REF !== "refs/heads/main"
  || process.env.GITHUB_EVENT_NAME !== "workflow_dispatch") throw new Error("the upload requires the exact manual main workflow");
const sha = requiredEnv("GITHUB_SHA");
if (!/^[a-f0-9]{40}$/.test(sha)) throw new Error("GITHUB_SHA must be a full commit SHA");
const runId = requiredEnv("GITHUB_RUN_ID");
const runAttempt = requiredEnv("GITHUB_RUN_ATTEMPT");
if (!/^[1-9]\d{0,19}$/.test(runId) || !/^[1-9]\d{0,19}$/.test(runAttempt)) {
  throw new Error("GITHUB_RUN_ID and GITHUB_RUN_ATTEMPT must identify the upload workflow run");
}
const accountId = requiredEnv("CLOUDFLARE_ACCOUNT_ID");
if (!/^[a-f0-9]{32}$/.test(accountId)) throw new Error("CLOUDFLARE_ACCOUNT_ID is invalid");
requiredEnv("CLOUDFLARE_API_TOKEN");
const handoffPath = requiredEnv("VOICE_LINE_UPLOAD_RECEIPT");
const workerId = exactWorkerId(parseCfJson(runCf(["-q", "workers", "get", "--worker", releaseWorker])));

const tag = `git-${sha}-r${runId}-a${runAttempt}`;
const message = `git ${sha}`;
const temp = await mkdtemp(resolve(tmpdir(), "voice-line-upload-"));
try {
  const output = resolve(temp, "cf-output.jsonl");
  runCf(["-q", "workers", "versions", "create", "--prebuilt", "--worker", releaseWorker, "--tag", tag, "--message", message],
    { ...process.env, WRANGLER_OUTPUT_FILE_PATH: output });
  const lines = (await readFile(output, "utf8")).trim().split("\n");
  const versionId = uploadedVersionId(lines.map((line) => JSON.parse(line)));
  const versions = parseVersionItems(runCf(["-q", "workers", "versions", "list", "--worker", workerId, "--per-page", "100"]));
  requireExactVersion(versions, versionId, sha, runId, runAttempt);
  const accountHash = createHash("sha256").update(accountId).digest("hex");
  await writeFile(handoffPath, JSON.stringify({
    worker: releaseWorker, workerIdHash: createHash("sha256").update(workerId).digest("hex"), accountHash, versionId, sourceSha: sha, uploadRunId: runId, uploadRunAttempt: runAttempt,
  }) + "\n", { mode: 0o600, flag: "wx" });
  console.log(`Uploaded candidate version ${versionId} from ${sha}, run ${runId} (attempt ${runAttempt}). No traffic changed.`);
} finally {
  await rm(temp, { recursive: true, force: true });
}

function runCf(args, env = process.env) {
  const binary = resolve(workerDir, "node_modules/cf/bin/cf");
  try {
    return execFileSync(process.execPath, [binary, ...args], {
      cwd: workerDir, env, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"],
    });
  } catch {
    throw new Error("cf upload or verification failed; inspect provider state before retrying");
  }
}

function requiredEnv(name) {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required`);
  return value;
}
