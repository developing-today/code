// Deploys the pkarr Worker through the cf versions flow (the #1828
// deployment standard's first slice):
//
//   cf workers versions create     upload a new version, stamped with the
//                                  git SHA (--tag git-<sha12>, full SHA in
//                                  --message); nothing goes live yet
//   GET .../versions?deployable=true
//                                  pre-check the new version is deployable
//   cf workers deployments create  put that version at 100%
//
// Versions and deployments are separate objects, so rollback is the same
// machinery run at an older version id: --deploy-version <id> creates a
// deployment at an existing version without uploading anything.
//
// One cf gap: versions create cannot create a brand-new Worker, so the
// first-ever deploy bootstraps through `cf deploy` (same tag/message);
// see the bootstrap note at the deploy step below.
//
// Usage:
//   deploy.mjs [--dry-run]                 upload + deploy HEAD
//   deploy.mjs --deploy-version <id>       deploy (or roll back to) an
//                                          existing version at 100%
//
// Needs CLOUDFLARE_ACCOUNT_ID and BWS_ACCESS_TOKEN; deploy.sh sets them.
// The API token comes from Bitwarden by name (default
// cloudflare.api-token-day-to-day; override with PKARR_CF_TOKEN_SECRET).
// No secret is ever written to disk or the Nix store.
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const WORKER = "pkarr";
const WORKER_URL = "https://pkarr.developing-today.workers.dev";

const args = process.argv.slice(2);
const dryRun = args.includes("--dry-run");
const deployVersionAt = args.indexOf("--deploy-version");
const deployVersion = deployVersionAt >= 0 ? args[deployVersionAt + 1] : null;
if (deployVersionAt >= 0 && !deployVersion) {
  throw new Error("--deploy-version needs a version id");
}

const accountId = requireEnv("CLOUDFLARE_ACCOUNT_ID");
const tokenSecretName = process.env.PKARR_CF_TOKEN_SECRET ?? "cloudflare.api-token-day-to-day";
const secrets = JSON.parse(execFileSync("bws", ["secret", "list", "-o", "json"], { encoding: "utf8" }));
const apiToken = secrets.find((secret) => secret.key === tokenSecretName)?.value;
if (!apiToken) {
  throw new Error(`no Bitwarden secret named ${tokenSecretName}`);
}
const env = { ...process.env, CLOUDFLARE_API_TOKEN: apiToken, CLOUDFLARE_ACCOUNT_ID: accountId };

// cf resolves the project's `cf/config` import from the project's own
// node_modules, so a fresh checkout needs its dev dependencies installed.
if (!existsSync(join(here, "node_modules"))) {
  console.log("node_modules missing; running npm ci");
  // The lockfile's resolved URLs are canonical (registry.npmjs.org). A
  // user-level npm registry mirror would otherwise rewrite them
  // (replace-registry-host) and can lag or 404 current tarballs, so pin
  // host replacement off for this install.
  execFileSync("npm", ["ci", "--no-audit", "--no-fund"], {
    cwd: here,
    stdio: "inherit",
    env: { ...process.env, npm_config_replace_registry_host: "false" },
  });
}

if (deployVersion !== null) {
  createDeployment(deployVersion);
  reportLive();
  process.exit(0);
}

const sha = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const tag = `git-${sha.slice(0, 12)}`;
const message = `deploy ${sha} via nix run .#pkarr-deploy`;

// cf gap (recorded on #1829): `cf workers versions create` cannot
// create a brand-new Worker — it refuses and tells you to run the
// deploy command first. So the very first deploy bootstraps with
// `cf deploy`, which creates the Worker's first version (with the
// same tag/message) and its first deployment in one step. Every
// later deploy uses the versions flow below.
const bootstrapping = !workerExists();
if (bootstrapping) {
  const deployArgs = ["deploy", "--worker", WORKER, "--tag", tag, "--message", message];
  if (dryRun) deployArgs.push("--dry-run");
  console.log(`worker ${WORKER} does not exist yet; bootstrapping with cf deploy (tag ${tag})`);
  runCf(deployArgs, { stdio: "inherit" });
  if (dryRun) {
    console.log("dry run: config builds, nothing uploaded");
    process.exit(0);
  }
} else {
  const createArgs = [
    "workers", "versions", "create",
    "--worker", WORKER,
    "--tag", tag,
    "--message", message,
  ];
  if (dryRun) createArgs.push("--dry-run");
  console.log(`uploading version (tag ${tag})`);
  runCf(createArgs, { stdio: "inherit" });
  if (dryRun) {
    console.log("dry run: config builds, nothing uploaded");
    process.exit(0);
  }
}

// Re-deploying an unchanged tree uploads a second version under the
// same tag (uploads are not deduplicated), so pick the newest match.
const version = listVersions()
  .filter((v) => v.annotations?.["workers/tag"] === tag)
  .sort((a, b) => createdOn(b) - createdOn(a))[0];
if (!version) {
  throw new Error(`uploaded version with tag ${tag} not found in versions list`);
}
console.log(`uploaded version ${version.id}`);

await assertDeployable(version.id);
if (bootstrapping) {
  console.log(`bootstrap deployment already live at version ${version.id}`);
} else {
  createDeployment(version.id);
}
reportLive();
console.log(`live: ${WORKER_URL} at version ${version.id} (${tag})`);

function workerExists() {
  try {
    execFileSync("cf", ["-q", "workers", "get", WORKER], {
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

function createDeployment(versionId) {
  const versions = JSON.stringify([{ version_id: versionId, percentage: 100 }]);
  console.log(`creating deployment at version ${versionId} (100%)`);
  // cf requires --strategy explicitly in non-interactive runs.
  runCf(
    ["workers", "deployments", "create", "--worker", WORKER, "--strategy", "percentage", "--versions", versions],
    { stdio: "inherit" },
  );
}

// The versions/deployments research pre-check: the API can filter the
// version list to deployable versions only. If the new version is not in
// that set, deploying it would fail — stop here with a clear message.
async function assertDeployable(versionId) {
  const url = `https://api.cloudflare.com/client/v4/accounts/${accountId}/workers/scripts/${WORKER}/versions?deployable=true`;
  const response = await fetch(url, { headers: { authorization: `Bearer ${apiToken}` } });
  if (!response.ok) {
    console.log(`warning: deployable pre-check returned HTTP ${response.status}; continuing`);
    return;
  }
  const body = await response.json();
  // The versions endpoint paginates: result is { items: [...] }, not a
  // bare array (verified against the live account, 2026-10-31).
  const items = Array.isArray(body.result) ? body.result : (body.result?.items ?? []);
  const ids = items.map((v) => v.id);
  if (!ids.includes(versionId)) {
    throw new Error(`version ${versionId} is not in the deployable set; refusing to create a deployment`);
  }
}

function createdOn(version) {
  const raw = version.metadata?.created_on ?? version.created_on ?? version.createdOn ?? 0;
  const parsed = typeof raw === "number" ? raw : Date.parse(raw);
  return Number.isFinite(parsed) ? parsed : 0;
}

function listVersions() {
  const out = runCf(["workers", "versions", "list", "--worker-id", WORKER], { capture: true });
  const parsed = parseJsonOutput(out);
  if (Array.isArray(parsed)) return parsed;
  if (Array.isArray(parsed?.result)) return parsed.result;
  if (Array.isArray(parsed?.versions)) return parsed.versions;
  throw new Error(`unexpected versions list output: ${out.slice(0, 200)}`);
}

function reportLive() {
  const out = runCf(["workers", "deployments", "list", "--worker", WORKER], { capture: true });
  console.log(`deployments:\n${out.trim()}`);
}

function runCf(cfArgs, { capture = false, stdio } = {}) {
  return execFileSync("cf", ["-q", ...cfArgs], {
    cwd: here,
    env,
    encoding: "utf8",
    stdio: stdio ?? (capture ? ["inherit", "pipe", "inherit"] : "inherit"),
  });
}

// cf prints JSON by default, but progress lines can precede it; take the
// last JSON value in the output.
function parseJsonOutput(out) {
  const text = out.trim();
  try {
    return JSON.parse(text);
  } catch {
    for (let i = text.indexOf("{"); i >= 0; i = text.indexOf("{", i + 1)) {
      try {
        return JSON.parse(text.slice(i));
      } catch { /* keep scanning */ }
    }
    for (let i = text.indexOf("["); i >= 0; i = text.indexOf("[", i + 1)) {
      try {
        return JSON.parse(text.slice(i));
      } catch { /* keep scanning */ }
    }
    throw new Error(`could not parse cf JSON output: ${text.slice(0, 200)}`);
  }
}

function requireEnv(name) {
  const value = process.env[name];
  if (!value) {
    throw new Error(`${name} is not set; run deploy.sh`);
  }
  return value;
}
