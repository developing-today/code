// Deploys the developing-today-backup Worker (github-backup webhook
// receiver) through the wrangler versions flow, following the shape of
// the repo's Cloudflare Worker deployment standard
// (doc/2026-10-31T23-00-00Z_design_cloudflare_worker_deployment_standard.md):
// upload a version stamped with the git SHA first, then create a
// deployment at 100% as a separate step — so rollback is the same
// machinery run at an older version id (--deploy-version <id>).
//
// Tool gap (recorded on #3215): the standard's deploy tool is the cf
// CLI, whose flow is built around a TypeScript cf/config project.
// This Worker is a dependency-free plain-JS worker configured by
// wrangler.toml, so the deploy script drives wrangler's versions
// commands (versions upload / versions deploy) instead — the
// standard's documented fallback where cf's surface does not fit.
//
// Usage:
//   deploy.mjs [--dry-run]                 validate, upload + deploy HEAD
//   deploy.mjs --deploy-version <id>       deploy (or roll back to) an
//                                          existing version at 100%
//                                          (--dry-run is rejected with
//                                          --deploy-version, #3474)
//
// Needs CLOUDFLARE_ACCOUNT_ID and BWS_ACCESS_TOKEN; deploy.sh sets
// them. The API token comes from Bitwarden by name (default
// cloudflare.api-token-day-to-day; override with GHB_CF_TOKEN_SECRET).
// After a live deploy, the Worker secrets DRAIN_BEARER and
// GITHUB_WEBHOOK_SECRET are (re)set from Bitwarden by name
// (github-app.backup-worker-bearer / github-app.backup-webhook-secret,
// see ../config.json). No secret is ever printed or written to disk.
import { execFileSync } from "node:child_process";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const WORKER = "developing-today-backup";
const WORKER_URL = "https://developing-today-backup.developing-today.workers.dev";

const args = process.argv.slice(2);
const dryRun = args.includes("--dry-run");
const deployVersionAt = args.indexOf("--deploy-version");
const deployVersion = deployVersionAt >= 0 ? args[deployVersionAt + 1] : null;
if (deployVersionAt >= 0 && !deployVersion) {
  throw new Error("--deploy-version needs a version id");
}
if (dryRun && deployVersion !== null) {
  // #3474: the deployVersion branch runs before the dryRun branch and
  // createDeployment() invokes wrangler versions deploy WITHOUT
  // --dry-run, so "--dry-run --deploy-version <id>" would perform a
  // real 100% deployment while looking like a preview. There is no
  // non-deploying preview of a versions deployment, so reject the
  // combination outright — before any bws/wrangler invocation.
  throw new Error(
    "--dry-run cannot be combined with --deploy-version: a dry run never deploys, " +
    "but --deploy-version always deploys at 100%. Drop --dry-run to roll back for real."
  );
}

requireEnv("CLOUDFLARE_ACCOUNT_ID");
const tokenSecretName = process.env.GHB_CF_TOKEN_SECRET ?? "cloudflare.api-token-day-to-day";
const secrets = JSON.parse(execFileSync("bws", ["secret", "list", "-o", "json"], { encoding: "utf8" }));
const byName = (name) => secrets.find((secret) => secret.key === name)?.value;
const apiToken = byName(tokenSecretName);
if (!apiToken) {
  throw new Error(`no Bitwarden secret named ${tokenSecretName}`);
}
const env = { ...process.env, CLOUDFLARE_API_TOKEN: apiToken };

if (deployVersion !== null) {
  createDeployment(deployVersion);
  reportLive();
  process.exit(0);
}

if (dryRun) {
  // wrangler deploy --dry-run builds + validates the config and exits
  // without uploading anything.
  runWrangler(["deploy", "--dry-run"], { stdio: "inherit" });
  console.log("dry run: config builds, nothing uploaded");
  process.exit(0);
}

const sha = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const tag = `git-${sha.slice(0, 12)}`;
const message = `deploy ${sha} via nix run .#developing-today-backup-deploy`;

// Bootstrap: versions upload needs an existing Worker. If this name
// has never been deployed (e.g. the 2026-10-31 rename from
// hatch-repo-backup-webhook), create it with a plain deploy first;
// the tagged versions flow below then proceeds as normal.
if (!workerExists()) {
  console.log(`worker ${WORKER} does not exist yet; bootstrapping with wrangler deploy`);
  runWrangler(["deploy"], { stdio: "inherit" });
}

console.log(`uploading version (tag ${tag})`);
runWrangler(["versions", "upload", "--name", WORKER, "--tag", tag, "--message", message], { stdio: "inherit" });

// Uploads are not deduplicated; pick the newest version with our tag.
const version = listVersions()
  .filter((v) => v.annotations?.["workers/tag"] === tag)
  .sort((a, b) => createdOn(b) - createdOn(a))[0];
if (!version) {
  throw new Error(`uploaded version with tag ${tag} not found in versions list`);
}
console.log(`uploaded version ${version.id}`);

const live = liveVersionIds();
if (live.includes(version.id)) {
  console.log(`version ${version.id} is already the live deployment (bootstrap)`);
} else {
  createDeployment(version.id);
}
setWorkerSecrets();
reportLive();
console.log(`live: ${WORKER_URL} at version ${version.id} (${tag})`);

function workerExists() {
  try {
    execFileSync("wrangler", ["versions", "list", "--name", WORKER, "--json"], {
      cwd: here,
      env,
      encoding: "utf8",
      stdio: ["inherit", "pipe", "pipe"],
    });
    return true;
  } catch {
    return false;
  }
}

function listVersions() {
  const out = runWrangler(["versions", "list", "--name", WORKER, "--json"], { capture: true });
  const parsed = JSON.parse(out);
  if (!Array.isArray(parsed)) throw new Error(`unexpected versions list output: ${out.slice(0, 200)}`);
  return parsed;
}

function liveVersionIds() {
  try {
    const out = runWrangler(["deployments", "list", "--name", WORKER, "--json"], { capture: true });
    const parsed = JSON.parse(out);
    const deployments = Array.isArray(parsed) ? parsed : [];
    const latest = deployments.sort((a, b) => createdOn(b) - createdOn(a))[0];
    return (latest?.versions ?? []).filter((v) => v.percentage === 100).map((v) => v.version_id);
  } catch {
    return [];
  }
}

function createDeployment(versionId) {
  console.log(`creating deployment at version ${versionId} (100%)`);
  runWrangler(["versions", "deploy", `${versionId}@100%`, "--name", WORKER, "--yes"], { stdio: "inherit" });
}

// (Re)set the Worker secrets from Bitwarden by name, so a renamed or
// fresh Worker is fully wired in the same run. Values pass through
// wrangler's stdin only; nothing is printed.
function setWorkerSecrets() {
  const wanted = [
    ["DRAIN_BEARER", "github-app.backup-worker-bearer"],
    ["GITHUB_WEBHOOK_SECRET", "github-app.backup-webhook-secret"],
  ];
  for (const [envName, secretName] of wanted) {
    const value = byName(secretName);
    if (!value) {
      console.log(`warning: no Bitwarden secret named ${secretName}; skipping ${envName}`);
      continue;
    }
    execFileSync("wrangler", ["secret", "put", envName, "--name", WORKER], {
      cwd: here,
      env,
      input: value,
      encoding: "utf8",
      stdio: ["pipe", "inherit", "inherit"],
    });
    console.log(`set Worker secret ${envName} (from Bitwarden: ${secretName})`);
  }
}

function reportLive() {
  const out = runWrangler(["deployments", "list", "--name", WORKER], { capture: true });
  console.log(`deployments:\n${out.trim()}`);
}

function createdOn(entry) {
  const raw = entry?.metadata?.created_on ?? entry?.created_on ?? 0;
  const parsed = typeof raw === "number" ? raw : Date.parse(raw);
  return Number.isFinite(parsed) ? parsed : 0;
}

function runWrangler(wranglerArgs, { capture = false, stdio } = {}) {
  return execFileSync("wrangler", wranglerArgs, {
    cwd: here,
    env,
    encoding: "utf8",
    stdio: stdio ?? (capture ? ["inherit", "pipe", "inherit"] : "inherit"),
  });
}

function requireEnv(name) {
  if (!process.env[name]) throw new Error(`${name} is not set; run deploy.sh`);
}
