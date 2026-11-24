// Deploys the queue-transport-alert Worker (code#6369) with the `cf` CLI's
// versions flow, the repo's Worker deployment standard (as pkarr-deploy):
//
//   cf workers versions create     upload a version stamped with the git
//                                  SHA (--tag git-<sha12>), its secrets
//                                  through a pipe; nothing goes live yet
//   cf workers deployments create  put that version at 100%
//   cf workers triggers deploy     apply the cron schedule from the config
//
// then checks the latest deployment serves that version and that
// /status.json answers. Rollback is the same machinery at an older id:
// --deploy-version <id> deploys an existing version (which carries its
// own secrets) without uploading anything.
//
// Usage:
//   deploy.mjs [--dry-run]                  build + upload + deploy HEAD
//   deploy.mjs --deploy-version <id>        deploy or roll back to <id>
//   deploy.mjs --deploy-version <id> --dry-run   preview; creates nothing
//
// The Worker package comes from the flake (QUEUE_TRANSPORT_ALERT_PKG) and
// cf/config plus wrangler (cf's build bridge) from the Nix store
// (QTA_CF_MODULE, QTA_WRANGLER_MODULE), so a deploy runs no npm install.
// Needs CLOUDFLARE_ACCOUNT_ID and BWS_ACCESS_TOKEN; deploy.sh sets them.
// Every value comes from Bitwarden by name:
//   QTA_CF_TOKEN_SECRET       (cloudflare.api-token-day-to-day)   cf's API token
//   QTA_ACCESS_ID_SECRET      (cloudflare.access.queue-broker-client-id)
//   QTA_ACCESS_SECRET_SECRET  (cloudflare.access.queue-broker-client-secret)
//   QTA_GITHUB_TOKEN_SECRET   (github.developing-today-code-automation)
//   QTA_TELEGRAM_SECRET       (openclaw.telegram-bot-token)       Telegram push
// and the broker's agent token (for the post-outage queue task) from the
// file the fleet queue CLI reads, QTA_BROKER_TOKEN_FILE (default
// ~/.config/queue-broker/token). A secret left out of an upload inherits
// the previous version's value. No secret is printed or written to disk.
import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync } from "node:fs";
import { setDefaultAutoSelectFamilyAttemptTimeout } from "node:net";
import { homedir, tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { listItems, newestWithTag, parseArgs, parseJsonOutput, servingCheck, singleServingVersion, ghaUploadIdentity, uploadedVersionId, hasExactUpload } from "./deploy-lib.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const WORKER = "queue-transport-alert";
const WORKER_URL = "https://queue-transport-alert.developing-today.workers.dev";
const BROWSER_UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

let dryRun;
let deployVersion;
try {
  ({ dryRun, deployVersion } = parseArgs(process.argv.slice(2)));
} catch (error) {
  console.error(`deploy.mjs: ${error.message}`);
  process.exit(2);
}

if (!process.env.CLOUDFLARE_ACCOUNT_ID) throw new Error("CLOUDFLARE_ACCOUNT_ID is not set; run deploy.sh");
const gha = process.env.QTA_GHA === "1";
if (gha && dryRun && deployVersion === null) throw new Error("GHA upload does not accept --dry-run");
if (gha && !process.env.CLOUDFLARE_API_TOKEN) throw new Error("CLOUDFLARE_API_TOKEN is required in GHA mode");
const names = {
  cf: process.env.QTA_CF_TOKEN_SECRET ?? "cloudflare.api-token-day-to-day",
  accessId: process.env.QTA_ACCESS_ID_SECRET ?? "cloudflare.access.queue-broker-client-id",
  accessSecret: process.env.QTA_ACCESS_SECRET_SECRET ?? "cloudflare.access.queue-broker-client-secret",
  github: process.env.QTA_GITHUB_TOKEN_SECRET ?? "github.developing-today-code-automation",
  telegram: process.env.QTA_TELEGRAM_SECRET ?? "openclaw.telegram-bot-token",
};
const vault = gha ? [] : JSON.parse(execFileSync("bws", ["secret", "list", "-o", "json"], { encoding: "utf8" }));
const byName = (name) => {
  const value = vault.find((s) => s.key === name)?.value;
  if (!value) throw new Error(`no Bitwarden secret named ${name}`);
  return value.replace(/\r?\n$/, "");
};
const env = { ...process.env, CLOUDFLARE_API_TOKEN: gha ? process.env.CLOUDFLARE_API_TOKEN : byName(names.cf) };

// Stage a writable copy (the store path is read-only and cf writes
// .cloudflare/ build output next to the config) with node_modules
// pointing at the Nix-built cf and wrangler.
const pkgDir = process.env.QUEUE_TRANSPORT_ALERT_PKG ?? here;
const stage = mkdtempSync(join(tmpdir(), "queue-transport-alert-deploy-"));
cpSync(pkgDir, stage, { recursive: true, filter: (src) => !src.includes("node_modules") });
// Copies of store paths keep their read-only modes; make the stage
// writable so it can be removed afterwards.
execFileSync("chmod", ["-R", "u+w", stage]);
process.on("exit", () => {
  try {
    rmSync(stage, { recursive: true, force: true });
  } catch (error) {
    console.warn(`could not remove ${stage}: ${error.message}`);
  }
});
if (!existsSync(join(stage, "cloudflare.config.ts"))) {
  throw new Error(`no cloudflare.config.ts in the Worker package at ${pkgDir}`);
}
mkdirSync(join(stage, "node_modules"), { recursive: true });
for (const [name, variable] of [["cf", "QTA_CF_MODULE"], ["wrangler", "QTA_WRANGLER_MODULE"]]) {
  const target = process.env[variable] ?? join(here, "node_modules", name);
  if (!existsSync(target)) throw new Error(`${variable} is not set and ${target} does not exist; run deploy.sh via nix run`);
  symlinkSync(target, join(stage, "node_modules", name));
}

function runCf(args, { capture = false, outputFile } = {}) {
  return execFileSync("cf", ["-q", ...args], {
    cwd: stage,
    env: outputFile ? { ...env, WRANGLER_OUTPUT_FILE_PATH: outputFile } : env,
    encoding: "utf8",
    stdio: ["inherit", capture ? "pipe" : "inherit", "inherit"],
  });
}

// cf reads --secrets-file with a plain file read, which fails on this
// process's stdin pipe. Hand it a fresh pipe instead: bash's process
// substitution copies our stdin (the secrets JSON) into /dev/fd/N, and
// cf itself gets /dev/null, so the values never touch disk or argv.
function runCfWithSecrets(args, secrets) {
  execFileSync("bash", ["-c", 'cf -q "$@" --secrets-file <(cat) </dev/null', "cf", ...args], {
    cwd: stage,
    env,
    input: JSON.stringify(secrets),
    stdio: ["pipe", "inherit", "inherit"],
  });
}

function createDeployment(versionId) {
  console.log(`creating deployment at version ${versionId} (100%)`);
  const versions = JSON.stringify([{ version_id: versionId, percentage: 100 }]);
  runCf(["workers", "deployments", "create", "--worker", WORKER, "--strategy", "percentage", "--versions", versions]);
}

async function verify(versionId) {
  const deployments = listItems(
    parseJsonOutput(runCf(["workers", "deployments", "list", "--worker", WORKER], { capture: true })),
    "deployments",
  );
  const serving = servingCheck(deployments, versionId);
  if (!serving.ok) {
    throw new Error(`post-deploy check failed: expected version ${versionId} at 100%, latest deployment serves ${serving.serving}`);
  }
  const url = `${WORKER_URL}/status.json`;
  // Node tries each address for only 250 ms by default; on the laptop's
  // network that timed out every address while curl got through.
  setDefaultAutoSelectFamilyAttemptTimeout(5_000);
  let status;
  for (let attempt = 1; ; attempt += 1) {
    try {
      const response = await fetch(url, { headers: { "user-agent": BROWSER_UA }, signal: AbortSignal.timeout(20_000) });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      status = await response.json();
      break;
    } catch (error) {
      if (attempt >= 3) {
        throw new Error(`version ${versionId} is deployed, but ${url} did not answer: ${error.cause?.message ?? error.message}`);
      }
      await new Promise((resolve) => setTimeout(resolve, 3_000));
    }
  }
  console.log(`verified: version ${versionId} serves 100%; ${url} reports the broker ${status.status ?? "unknown"}`);
}

if (gha && deployVersion === null) {
  const currentDeployments = listItems(parseJsonOutput(runCf(["workers", "deployments", "list", "--worker", WORKER], { capture: true })), "deployments");
  const baselineVersion = singleServingVersion(currentDeployments);
  if (!baselineVersion) {
    throw new Error("cannot upload code-only version: no single 100% serving baseline version to inherit runtime bindings from");
  }
  // Inherits against the latest uploaded version, not necessarily the serving one.
  const priorVersions = listItems(parseJsonOutput(runCf(["workers", "versions", "list", "--worker-id", WORKER, "--per-page", "1"], { capture: true })), "versions");
  if (priorVersions[0]?.id !== baselineVersion) {
    throw new Error("latest upload is not the serving baseline; refusing runtime binding inheritance");
  }
  const sha = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
  if (sha !== process.env.QTA_SOURCE_SHA) throw new Error("checkout does not match dispatch SHA");
  if (!process.env.QTA_HANDOFF_FILE) throw new Error("GHA upload requires a handoff path");
  const identity = ghaUploadIdentity({ sha, runId: process.env.GITHUB_RUN_ID, attempt: process.env.GITHUB_RUN_ATTEMPT });
  const outputFile = join(stage, "cf-versions-upload-result.json");
  runCf(["workers", "versions", "create", "--worker", WORKER, "--tag", identity.tag, "--message", identity.message], { outputFile });
  const versionId = uploadedVersionId(readFileSync(outputFile, "utf8"), WORKER);
  const versions = listItems(parseJsonOutput(runCf(["workers", "versions", "list", "--worker-id", WORKER], { capture: true })), "versions");
  if (!hasExactUpload(versions, versionId, identity)) throw new Error("provider did not read back the exact created version and provenance");
  const current = listItems(parseJsonOutput(runCf(["workers", "deployments", "list", "--worker", WORKER], { capture: true })), "deployments");
  if (singleServingVersion(current) !== baselineVersion) throw new Error("serving baseline changed during upload");
  const handoff = {
    worker: WORKER,
    source_sha: sha,
    tag: identity.tag,
    version_id: versionId,
    upload_run_id: process.env.GITHUB_RUN_ID,
    upload_run_attempt: process.env.GITHUB_RUN_ATTEMPT,
    upload_event: process.env.GITHUB_EVENT_NAME,
    upload_ref: process.env.GITHUB_REF,
    upload_workflow_path: process.env.GITHUB_WORKFLOW_REF?.split("@")[0]?.replace(`${process.env.GITHUB_REPOSITORY}/`, ""),
    baseline_version_id: baselineVersion,
  };
  if (!handoff.upload_run_id || !handoff.upload_run_attempt || !process.env.QTA_HANDOFF_FILE) {
    throw new Error("GHA upload requires GITHUB_RUN_ID, GITHUB_RUN_ATTEMPT and QTA_HANDOFF_FILE");
  }
  await (await import("node:fs/promises")).writeFile(process.env.QTA_HANDOFF_FILE, `${JSON.stringify(handoff, null, 2)}\n`, { mode: 0o600 });
  console.log(`uploaded version ${versionId}; handoff manifest saved`);
  process.exit(0);
}

if (deployVersion !== null) {
  if (gha) {
    const handoff = JSON.parse(readFileSync(process.env.QTA_HANDOFF_FILE ?? "", "utf8"));
    const identity = ghaUploadIdentity({ sha: handoff.source_sha, runId: handoff.upload_run_id, attempt: handoff.upload_run_attempt });
    if (handoff.worker !== WORKER || handoff.version_id !== deployVersion || handoff.source_sha !== process.env.QTA_SOURCE_SHA ||
        handoff.upload_run_id !== process.env.QTA_UPLOAD_RUN_ID || handoff.tag !== identity.tag) {
      throw new Error("handoff manifest does not bind this version to the expected Worker, source SHA, tag and upload run");
    }
    const versions = listItems(parseJsonOutput(runCf(["workers", "versions", "list", "--worker-id", WORKER], { capture: true })), "versions");
    if (!hasExactUpload(versions, deployVersion, identity)) {
      throw new Error("requested Worker version lacks the verified immutable upload provenance");
    }
    const deployments = listItems(parseJsonOutput(runCf(["workers", "deployments", "list", "--worker", WORKER], { capture: true })), "deployments");
    if (singleServingVersion(deployments) !== handoff.baseline_version_id) {
      throw new Error("serving baseline changed since code-only upload; refusing promotion because inherited runtime bindings cannot be established");
    }
  }
  if (dryRun) {
    console.log(`dry run: would deploy ${WORKER} at existing version ${deployVersion} (100%); nothing created`);
    process.exit(0);
  }
  createDeployment(deployVersion);
  await verify(deployVersion);
  console.log(`live: ${WORKER_URL} at version ${deployVersion}`);
  process.exit(0);
}

const sha = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const tag = `git-${sha.slice(0, 12)}`;
const message = `deploy ${sha} via nix run .#queue-transport-alert-deploy`;

if (dryRun) {
  runCf(["workers", "versions", "create", "--worker", WORKER, "--tag", tag, "--message", message, "--dry-run"]);
  console.log("dry run: config builds, nothing uploaded");
  process.exit(0);
}

const workerSecrets = {
  ACCESS_CLIENT_ID: byName(names.accessId),
  ACCESS_CLIENT_SECRET: byName(names.accessSecret),
  GITHUB_TOKEN: byName(names.github),
  TELEGRAM_BOT_TOKEN: byName(names.telegram),
};
const brokerTokenFile = process.env.QTA_BROKER_TOKEN_FILE ?? join(homedir(), ".config/queue-broker/token");
const brokerToken = existsSync(brokerTokenFile) ? readFileSync(brokerTokenFile, "utf8").trim() : "";
if (brokerToken) {
  workerSecrets.BROKER_AGENT_TOKEN = brokerToken;
} else {
  console.warn(`no broker token at ${brokerTokenFile}; BROKER_AGENT_TOKEN carries over from the previous version`);
}

console.log(`uploading version (tag ${tag}; secrets ${Object.keys(workerSecrets).join(", ")})`);
runCfWithSecrets(["workers", "versions", "create", "--worker", WORKER, "--tag", tag, "--message", message], workerSecrets);
const versions = listItems(
  parseJsonOutput(runCf(["workers", "versions", "list", "--worker-id", WORKER], { capture: true })),
  "versions",
);
const version = newestWithTag(versions, tag);
if (!version) throw new Error(`uploaded version with tag ${tag} not found in the versions list`);
console.log(`uploaded version ${version.id}`);

createDeployment(version.id);
console.log("applying cron trigger from cloudflare.config.ts");
runCf(["workers", "triggers", "deploy", "--worker", WORKER]);
await verify(version.id);
console.log(`live: ${WORKER_URL} at version ${version.id} (${tag}); roll back with --deploy-version <older id>`);
