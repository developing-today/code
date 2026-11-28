// Deploys the cold-store-rehydrator Worker through the wrangler
// versions flow, following the shape of the repo's Cloudflare Worker
// deployment standard
// (doc/2026-10-31T23-00-00Z_design_cloudflare_worker_deployment_standard.md):
// upload a version stamped with the git SHA first, then create a
// deployment at 100% as a separate step — so rollback is the same
// machinery run at an older version id (--deploy-version <id>).
//
// Issue #5767, phase 2: this replaces the manual REST PUT of the
// committed dist/ bundle (the old worker/README flow). The bundle is
// a flake package (package.nix, lockfile-pinned deps, esbuild build);
// deploy.nix passes its store path as COLD_STORE_REHYDRATOR_BUNDLE and
// this script stages it with wrangler.toml in a temp dir for the
// wrangler versions commands. Run directly from a checkout without
// the env var, it falls back to the checkout's own dist/worker.js.
// No deploy-time `npm install` happens on either path.
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
// cloudflare.api-token-day-to-day; override with CSR_CF_TOKEN_SECRET).
// After a live deploy, the Worker secrets REHYDRATOR_API_KEY,
// GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET, and GOOGLE_REFRESH_TOKEN
// are (re)set from Bitwarden by name (cold-store.rehydrator-api-key,
// google.cold-store-reaper.client-id / .client-secret /
// .refresh-token — the same Google grant the reaper uses). No secret
// is ever printed or written to disk.
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const WORKER = "cold-store-rehydrator";
// [Worker binding name, Bitwarden secret name] pairs the Worker needs
// to serve authenticated endpoints. A version without these bindings
// is not a sound rollback target (#3475).
const WORKER_SECRETS = [
  ["REHYDRATOR_API_KEY", "cold-store.rehydrator-api-key"],
  ["GOOGLE_CLIENT_ID", "google.cold-store-reaper.client-id"],
  ["GOOGLE_CLIENT_SECRET", "google.cold-store-reaper.client-secret"],
  ["GOOGLE_REFRESH_TOKEN", "google.cold-store-reaper.refresh-token"],
];
const WORKER_URL = "https://cold-store-rehydrator.developing-today.workers.dev";

// Stage the wrangler project: the committed wrangler.toml plus the
// built bundle at dist/worker.js (its `main`). Wrangler runs with
// this directory as cwd on every command below.
const bundleSource =
  process.env.COLD_STORE_REHYDRATOR_BUNDLE ?? join(here, "dist", "worker.js");
if (!existsSync(bundleSource)) {
  throw new Error(
    `Worker bundle not found at ${bundleSource}; build it with ` +
      "`nix build .#cold-store-rehydrator` (or `npm ci && npm run build` in a checkout)"
  );
}
const stage = mkdtempSync(join(tmpdir(), "cold-store-rehydrator-deploy-"));
mkdirSync(join(stage, "dist"));
copyFileSync(join(here, "wrangler.toml"), join(stage, "wrangler.toml"));
copyFileSync(bundleSource, join(stage, "dist", "worker.js"));

const args = process.argv.slice(2);
// Strict argument validation (#3287, phase 1 of #3352): every argument
// is accounted for before any bws/wrangler/git subprocess or deploy
// side effect. The only accepted invocations are `deploy.mjs`,
// `deploy.mjs --dry-run`, and `deploy.mjs --deploy-version <id>`.
// Anything else — a typo'd `--dry-rnu`, a stray positional, an unknown
// flag, a missing or flag-like --deploy-version value — aborts here
// with a usage error naming the offending argument (exit 2), following
// the sibling #3281 contract.
function usageError(message) {
  console.error(
    `${message}\nusage: deploy.mjs [--dry-run] | deploy.mjs --deploy-version <id>`
  );
  process.exit(2);
}
let dryRun = false;
let deployVersion = null;
for (let i = 0; i < args.length; i++) {
  const arg = args[i];
  if (arg === "--dry-run") {
    if (dryRun) usageError("duplicate argument: --dry-run");
    dryRun = true;
  } else if (arg === "--deploy-version") {
    if (deployVersion !== null) usageError("duplicate argument: --deploy-version");
    const value = args[i + 1];
    if (value === undefined || value === "") {
      usageError("--deploy-version needs a version id");
    }
    if (value.startsWith("-")) {
      usageError(`--deploy-version needs a version id, got flag-like value: ${value}`);
    }
    deployVersion = value;
    i++;
  } else if (arg.startsWith("-")) {
    usageError(`unrecognized argument: ${arg}`);
  } else {
    usageError(`unrecognized argument: ${arg}`);
  }
}
if (dryRun && deployVersion !== null) {
  // #3474: the deployVersion branch runs before the dryRun branch and
  // createDeployment() invokes wrangler versions deploy WITHOUT
  // --dry-run, so "--dry-run --deploy-version <id>" would perform a
  // real 100% deployment while looking like a preview. There is no
  // non-deploying preview of a versions deployment, so reject the
  // combination outright — before any bws/wrangler invocation.
  console.error(
    "--dry-run cannot be combined with --deploy-version: a dry run never deploys, " +
    "but --deploy-version always deploys at 100%. Drop --dry-run to roll back for real.\n" +
    "usage: deploy.mjs [--dry-run] | deploy.mjs --deploy-version <id>"
  );
  process.exit(2);
}

requireEnv("CLOUDFLARE_ACCOUNT_ID");
const tokenSecretName = process.env.CSR_CF_TOKEN_SECRET ?? "cloudflare.api-token-day-to-day";
const secrets = JSON.parse(execFileSync("bws", ["secret", "list", "-o", "json"], { encoding: "utf8" }));
const byName = (name) => secrets.find((secret) => secret.key === name)?.value;
const apiToken = byName(tokenSecretName);
if (!apiToken) {
  throw new Error(`no Bitwarden secret named ${tokenSecretName}`);
}
const env = { ...process.env, CLOUDFLARE_API_TOKEN: apiToken };

if (deployVersion !== null) {
  // #3475: deploying an existing version restores that version exactly,
  // bindings included. A version uploaded by the normal flow predates
  // the runtime-configuration step, so it carries no secret bindings —
  // and the configuration cannot be added after the fact: ordinary
  // `wrangler secret put` hard-errors when the latest version is not
  // the deployed one (the rollback state by definition), and
  // `wrangler versions secret put` patches the LATEST version, not the
  // selected one. So verify BEFORE switching traffic: refuse a target
  // missing the required secret bindings and name the remedy (roll
  // forward through the normal flow, which uploads, deploys, and
  // provisions, producing a configured version). Configured versions —
  // including every version the normal flow now advertises — deploy
  // as-is.
  verifyVersionConfigured(deployVersion);
  createDeployment(deployVersion);
  reportLive();
  console.log(
    `live: ${WORKER_URL} at version ${deployVersion} ` +
    `(rollback; required runtime configuration verified on the version)`
  );
  process.exit(0);
}

if (dryRun) {
  // wrangler deploy --dry-run builds + validates the config and exits
  // without uploading anything.
  runWrangler(["deploy", "--dry-run"], { stdio: "inherit" });
  console.log("dry run: config builds, nothing uploaded");
  process.exit(0);
}

const sha = execFileSync("git", ["rev-parse", "HEAD"], { cwd: here, encoding: "utf8" }).trim();
const tag = `git-${sha.slice(0, 12)}`;
const message = `deploy ${sha} via nix run .#cold-store-rehydrator-deploy`;

// Bootstrap: versions upload needs an existing Worker. This Worker
// predates the versions flow (it was deployed by manual REST PUT), so
// it exists; the bootstrap stays for a fresh account or a rename.
if (!workerExists()) {
  console.log(`worker ${WORKER} does not exist yet; bootstrapping with wrangler deploy`);
  // #15793: stamp the deployed commit as the VERSION var the public
  // GET /version route reads (bootstrap path; the versions upload
  // below stamps it the same way).
  runWrangler(["deploy", "--var", `VERSION:${sha}`], { stdio: "inherit" });
}

console.log(`uploading version (tag ${tag})`);
runWrangler(
  ["versions", "upload", "--name", WORKER, "--tag", tag, "--message", message, "--var", `VERSION:${sha}`],
  { stdio: "inherit" },
);

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
// #3475: `wrangler secret put` creates and deploys a NEW version that
// carries the runtime configuration; the uploaded version.id predates
// it and has no secret bindings. The version to advertise — and the
// only sound rollback target — is the final configured live version,
// re-read after provisioning, not the pre-secrets upload.
// If the re-read cannot name a live version (read/parse failure, empty
// deployments list, no version at 100%), the uploaded version must NOT
// be advertised in its place (#6507): fail loudly instead.
const configuredLive = liveVersionIds();
if (configuredLive.length === 0) {
  console.error(
    `error: could not determine the configured live version after provisioning secrets ` +
    `(deployments list was empty, unreadable, or had no version at 100%). ` +
    `Rollback target UNKNOWN: do NOT use the uploaded version ${version.id} (${tag}); it predates ` +
    `the secret bindings (#3475). Check \`wrangler deployments list --name ${WORKER}\` and ` +
    `use the configured version it shows.`
  );
  process.exit(1);
}
const liveId = configuredLive.join(", ");
console.log(`live: ${WORKER_URL} at version ${liveId} (${tag})`);
if (liveId !== version.id) {
  console.log(
    `note: uploaded version ${version.id} was superseded by configured version ${liveId}; ` +
    `use ${liveId} as the rollback target`
  );
}

function workerExists() {
  try {
    execFileSync("wrangler", ["versions", "list", "--name", WORKER, "--json"], {
      cwd: stage,
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

// Refuse to make live a version that lacks the required secret
// bindings. Reads the version's bindings via `wrangler versions view`;
// secret values are never fetched, only binding names.
function verifyVersionConfigured(versionId) {
  const out = runWrangler(["versions", "view", versionId, "--name", WORKER, "--json"], { capture: true });
  const parsed = JSON.parse(out);
  const bindings = parsed?.resources?.bindings ?? parsed?.bindings ?? [];
  const secretNames = new Set(
    bindings.filter((b) => b?.type === "secret_text").map((b) => b.name)
  );
  const missing = WORKER_SECRETS.map(([envName]) => envName).filter((name) => !secretNames.has(name));
  if (missing.length > 0) {
    throw new Error(
      `version ${versionId} lacks required runtime configuration (${missing.join(", ")} secret binding(s) missing); ` +
      `refusing to roll back to an unconfigured version. Roll forward instead: check out the version's git tag and ` +
      `run the normal deploy, which uploads, deploys, and provisions the secrets, producing a configured version.`
    );
  }
  console.log(`version ${versionId} carries the required secret bindings (${[...secretNames].join(", ")})`);
}

function createDeployment(versionId) {
  console.log(`creating deployment at version ${versionId} (100%)`);
  runWrangler(["versions", "deploy", `${versionId}@100%`, "--name", WORKER, "--yes"], { stdio: "inherit" });
}

// (Re)set the Worker secrets from Bitwarden by name, so a renamed or
// fresh Worker is fully wired in the same run. Each `secret put`
// creates a new Worker version carrying the binding, so after this
// runs the live version is a configured successor of the version that
// was deployed — callers must re-read the live version before
// advertising a rollback target (#3475). Values pass through
// wrangler's stdin only; nothing is printed.
function setWorkerSecrets() {
  for (const [envName, secretName] of WORKER_SECRETS) {
    const value = byName(secretName);
    if (!value) {
      console.log(`warning: no Bitwarden secret named ${secretName}; skipping ${envName}`);
      continue;
    }
    execFileSync("wrangler", ["secret", "put", envName, "--name", WORKER], {
      cwd: stage,
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
    cwd: stage,
    env,
    encoding: "utf8",
    stdio: stdio ?? (capture ? ["inherit", "pipe", "inherit"] : "inherit"),
  });
}

function requireEnv(name) {
  if (!process.env[name]) throw new Error(`${name} is not set; run deploy.sh`);
}
