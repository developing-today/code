// Deploys the message-bridge Worker through the cf versions flow, the
// same one pkgs/pkarr-worker uses:
//
//   cf workers versions create     upload a new version tagged with the
//                                  git SHA (git-<sha12>, full SHA in the
//                                  message); nothing goes live yet
//   GET .../versions?deployable=true
//                                  pre-check the new version is deployable
//                                  and still has its ACCESS_TOKEN
//   cf workers deployments create  put that version at 100%
//   then check the deployment serves it and /healthz says the token is set
//
// Every deploy leaves the previous versions in place, so a rollback is
// a deployment at an older version id (--deploy-version <id>), with no
// upload.
//
// The one exception is the very first deploy into an account: `cf
// workers versions create` can't create a Worker, so when the API says
// message-bridge doesn't exist yet (deploy-probe.mjs), a full deploy
// bootstraps with `cf deploy` (same tag, message and ACCESS_TOKEN),
// which uploads and deploys in one step. --code-only refuses there,
// since it has no access token to give the new Worker.
//
// Usage:
//   deploy.mjs                        full deploy from a checkout: upload
//                                     a version with ACCESS_TOKEN from
//                                     Bitwarden, deploy it
//   deploy.mjs --dry-run              build and check; upload nothing
//   deploy.mjs --code-only            the repo's deploy (the
//                                     message-bridge-deploy workflow):
//                                     CLOUDFLARE_API_TOKEN and
//                                     CLOUDFLARE_ACCOUNT_ID from the env,
//                                     no Bitwarden; the new version keeps
//                                     the deployed ACCESS_TOKEN, so the
//                                     connector URL doesn't change
//   deploy.mjs --deploy-version <id> [--allow-secret-change] [--dry-run]
//                                     deploy (roll back to) an existing
//                                     version at 100%; --dry-run previews.
//                                     Cloudflare blocks an older version
//                                     whose secrets have since changed;
//                                     --allow-secret-change deploys it
//                                     anyway (with that version's token)
//   deploy.mjs --print-urls           print the connector, page and API
//                                     URLs (they contain the token)
//
// Credentials: CLOUDFLARE_API_TOKEN from the env for --code-only and in
// GitHub Actions, otherwise from Bitwarden (MB_CF_TOKEN_SECRET, default
// cloudflare.api-token-day-to-day); CLOUDFLARE_ACCOUNT_ID from the env
// (deploy.sh decrypts it with sops). The access token is Bitwarden's
// message-bridge.access-token (MB_ACCESS_TOKEN_SECRET), created on the
// first full deploy (32 random bytes, base64url) in project
// MB_BWS_PROJECT. It reaches cf through --secrets-file, a 0600 file on
// /dev/shm (memory) that is removed as soon as the upload returns. To rotate it, edit or delete that secret and run a full
// deploy.
import { execFileSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import {
  chmodSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs, USAGE } from "./deploy-args.mjs";
import {
  CODE_ONLY_MARK,
  VersionListGap,
  classifyWorkerProbe,
  codeOnlyFingerprint,
  healthVersionProblem,
  isOwnDeployment,
  promoteOrRollBack,
  replacedOtherThan,
  strayBeforeCodeOnly,
} from "./deploy-probe.mjs";
import { tokenFingerprint } from "./src/auth.js";

const here = dirname(fileURLToPath(import.meta.url));
const WORKER = "message-bridge";
const MIN_TOKEN_LENGTH = 32;

let args;
try {
  args = parseArgs(process.argv.slice(2));
} catch (err) {
  console.error(`deploy.mjs: ${err.message}\n${USAGE}`);
  process.exit(2);
}
// deploy.sh checks the arguments this way before it touches Bitwarden.
if (process.env.MB_CHECK_ARGS_ONLY === "1") process.exit(0);

const names = {
  cf: process.env.MB_CF_TOKEN_SECRET ?? "cloudflare.api-token-day-to-day",
  access: process.env.MB_ACCESS_TOKEN_SECRET ?? "message-bridge.access-token",
};
const project = process.env.MB_BWS_PROJECT ?? "d370cd10-3d67-4b34-b61a-b4dc00753f97";

const pkgDir = process.env.MESSAGE_BRIDGE_PKG ?? here;
const wranglerConfig = readFileSync(join(pkgDir, "wrangler.jsonc"), "utf8");
const publicUrl = (wranglerConfig.match(/"PUBLIC_URL"\s*:\s*"([^"]+)"/)?.[1] ?? "").replace(/\/+$/, "");

let vault = null;
function fromVault(name) {
  vault ??= JSON.parse(execFileSync("bws", ["secret", "list", "-o", "json"], { encoding: "utf8" }));
  return vault.find((s) => s.key === name)?.value?.replace(/\r?\n$/, "");
}

function accessTokenFromVault() {
  const token = fromVault(names.access);
  if (token && token.length < MIN_TOKEN_LENGTH) {
    throw new Error(`Bitwarden secret ${names.access} is shorter than ${MIN_TOKEN_LENGTH} characters; the Worker would refuse it`);
  }
  return token;
}

if (args.mode === "print-urls") {
  const token = accessTokenFromVault();
  if (!token) throw new Error(`no Bitwarden secret named ${names.access}; deploy once to create it`);
  if (!publicUrl) throw new Error("no PUBLIC_URL in wrangler.jsonc");
  console.log(`connector: ${publicUrl}/mcp/${token}`);
  console.log(`page:      ${publicUrl}/p/${token}`);
  console.log(`api:       ${publicUrl}/api/${token}/messages`);
  process.exit(0);
}

// Credentials. --code-only, and any run in GitHub Actions, takes the API
// token from the environment and never reads Bitwarden. Everything else
// uses Bitwarden's token even when the shell has another
// CLOUDFLARE_API_TOKEN set, so a local deploy always runs with the
// token this script names.
const tokenFromEnv = args.mode === "code-only" || process.env.GITHUB_ACTIONS === "true";
if (tokenFromEnv && !process.env.CLOUDFLARE_API_TOKEN) {
  throw new Error("CLOUDFLARE_API_TOKEN is not set; --code-only and CI read it from the environment");
}
const accountId = process.env.CLOUDFLARE_ACCOUNT_ID;
if (!accountId) throw new Error("CLOUDFLARE_ACCOUNT_ID is not set; run deploy.sh");
const apiToken = tokenFromEnv ? process.env.CLOUDFLARE_API_TOKEN : fromVault(names.cf);
if (!apiToken) throw new Error(`no CLOUDFLARE_API_TOKEN and no Bitwarden secret named ${names.cf}`);
const env = { ...process.env, CLOUDFLARE_API_TOKEN: apiToken, CLOUDFLARE_ACCOUNT_ID: accountId };
const api = `https://api.cloudflare.com/client/v4/accounts/${accountId}/workers/scripts/${WORKER}`;

if (args.mode === "deploy-version") {
  const id = args.deployVersion;
  await assertDeployable(id);
  if (args.dryRun) {
    console.log(`dry run: would deploy ${WORKER} at version ${id} (100%); nothing changed`);
    process.exit(0);
  }
  const stage = stagePackage();
  try {
    createDeployment(stage, id, { bypassChecks: args.allowSecretChange });
  } catch (err) {
    if (args.allowSecretChange) throw err;
    throw new Error(
      `Cloudflare refused to deploy version ${id} (cf's error is above). If ACCESS_TOKEN or another secret changed ` +
        `since that version was uploaded, Cloudflare blocks rolling back to it; rerun with --allow-secret-change to ` +
        `deploy it anyway, knowing the connector then uses that version's token.`,
    );
  }
  await verifyLive(id);
  console.log(`live: ${publicUrl} at version ${id}`);
  process.exit(0);
}

const stage = stagePackage();
const sha = process.env.GITHUB_SHA || execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const tag = `git-${sha.slice(0, 12)}`;
const via = args.mode === "code-only" ? CODE_ONLY_MARK.slice("via ".length) : "nix run .#message-bridge-deploy";
// The run id makes this upload's message unique, so the version found
// after the upload is this run's and never another upload of the same
// SHA (a local deploy and the workflow can share a tag).
const runId = randomBytes(6).toString("hex");
const message = `deploy ${sha} via ${via} (run ${runId})`;
const annotate = ["--tag", tag, "--message", message];
const exists = await workerExists();
if (!exists && args.mode === "code-only") {
  throw new Error(
    `worker ${WORKER} does not exist yet; run the first deploy from a checkout (nix run .#message-bridge-deploy) so it gets its ACCESS_TOKEN`,
  );
}
if (exists && args.mode === "code-only") await assertTokenInheritsFromLive();
const uploadArgs = exists
  ? ["workers", "versions", "create", ...annotate]
  : ["deploy", "--worker", WORKER, ...annotate];

if (args.dryRun) {
  if (!exists) console.log(`worker ${WORKER} does not exist yet; a real deploy would bootstrap it with cf deploy`);
  runCf(stage, [...uploadArgs, "--dry-run"]);
  console.log("dry run: config builds, nothing uploaded");
  process.exit(0);
}

let secretsDir = null;
let expectFingerprint = null;
if (args.mode === "deploy") {
  let token = accessTokenFromVault();
  if (!token) {
    token = randomBytes(32).toString("base64url");
    execFileSync(
      "bws",
      [
        "secret",
        "create",
        "--note",
        "message-bridge Worker path token (guards /mcp, /p, /api). Rotate by editing and redeploying.",
        "-o",
        "none",
        "--",
        names.access,
        token,
        project,
      ],
      { stdio: ["ignore", "ignore", "inherit"] },
    );
    console.log(`created Bitwarden secret ${names.access}`);
  }
  // cf reads secrets only from a regular file (not /dev/stdin), so the
  // token goes in a 0600 file inside a 0700 directory on /dev/shm
  // (memory, not disk), removed as soon as the upload returns.
  secretsDir = mkdtempSync(join(existsSync("/dev/shm") ? "/dev/shm" : tmpdir(), "message-bridge-secrets-"));
  chmodSync(secretsDir, 0o700);
  const secretsFile = join(secretsDir, "secrets.json");
  writeFileSync(secretsFile, JSON.stringify({ ACCESS_TOKEN: token }), { mode: 0o600 });
  expectFingerprint = await tokenFingerprint(token);
  uploadArgs.push("--secrets-file", secretsFile);
}

console.log(
  exists
    ? `uploading version (tag ${tag})`
    : `worker ${WORKER} does not exist yet; bootstrapping with cf deploy (tag ${tag})`,
);
try {
  runCf(stage, uploadArgs);
} finally {
  if (secretsDir) rmSync(secretsDir, { recursive: true, force: true });
}

// The versions list can lag a just-finished upload by a few seconds.
let matches = [];
for (let attempt = 1; attempt <= 8 && matches.length === 0; attempt++) {
  if (attempt > 1) await new Promise((r) => setTimeout(r, 2500));
  matches = (await listVersions()).filter((v) => v.annotations?.["workers/message"] === message);
}
if (matches.length !== 1) {
  throw new Error(`expected exactly one version with this run's message, found ${matches.length}: ${message}`);
}
const version = matches[0];
console.log(`uploaded version ${version.id}`);
if (args.mode === "code-only") await assertTokenInheritsFromLive(version.id);
await assertDeployable(version.id);
// Read what is live right before promoting: its version is the one to
// put back if the new one fails verification or carries a different
// token, and a code-only version must keep its token fingerprint.
let previousId = null;
if (exists) {
  previousId = (await liveVersions())[0]?.version_id ?? null;
  if (args.mode === "code-only") {
    try {
      expectFingerprint = codeOnlyFingerprint(await readHealth());
    } catch (err) {
      err.message += `; version ${version.id} was left undeployed (a full deploy from a checkout fixes this)`;
      throw err;
    }
  }
}
if (exists) {
  // Filled in once verification sees this run's deployment, so a later
  // deployment of the same version (someone else's) isn't mistaken for it.
  const own = { deploymentId: null };
  await promoteOrRollBack({
    newId: version.id,
    previousId,
    promote: () => createDeployment(stage, version.id),
    verify: () => verifyLive(version.id, { expectFingerprint, requireVersion: true, replaced: previousId, own }),
    stillNewest: async () => isOwnDeployment((await deploymentsNewestFirst())[0], version.id, own.deploymentId),
    rollback: (id) => {
      console.error(`verification failed; putting version ${id} back live`);
      createDeployment(stage, id, { bypassChecks: true });
    },
  });
} else {
  // cf deploy already put the bootstrap version live; nothing to go back to.
  await verifyLive(version.id, { expectFingerprint, requireVersion: true });
}
console.log(`live: ${publicUrl} at version ${version.id} (${tag})`);
console.log(`roll back with: nix run .#message-bridge-deploy -- --deploy-version <older version id>`);

// --- helpers ---

// A code-only upload keeps ACCESS_TOKEN by inheriting it from the newest
// uploaded version, which is not always the live one: a full deploy that
// uploaded a rotated token and then failed before promotion leaves a
// newer version with a different token. So every version between the
// live one and this upload must itself be a code-only upload (which only
// ever carries a token forward), or this refuses. It runs before the
// upload (so a doomed upload never happens) and again after it, over the
// versions numbered before this run's version, so a full deploy that
// slipped in between the check and the upload can't be inherited
// either; then this run's version is left undeployed. Versions are
// numbered in upload order, so the check knows exactly which versions it
// must see and waits until the list shows them all.
async function assertTokenInheritsFromLive(uploadedId = null) {
  const liveId = (await liveVersions())[0]?.version_id;
  if (!liveId) throw new Error("could not find the live version; refusing a code-only deploy");
  // Retry while the list is missing a version number in the range: it
  // can lag an upload, and an unlisted version could be a full deploy.
  let stray = null;
  for (let attempt = 1; ; attempt++) {
    try {
      stray = strayBeforeCodeOnly(await listVersions(), liveId, uploadedId);
      break;
    } catch (err) {
      if (!(err instanceof VersionListGap) || attempt >= 8) {
        if (uploadedId && err instanceof VersionListGap) err.message += ` (version ${uploadedId} was uploaded but not deployed)`;
        throw err;
      }
      await new Promise((r) => setTimeout(r, 2500));
    }
  }
  if (stray) {
    throw new Error(
      `version ${stray.id} sits between the live version ${liveId} and this upload and isn't a code-only ` +
        `upload, so a code-only deploy could carry a different ACCESS_TOKEN` +
        (uploadedId ? ` (version ${uploadedId} was uploaded but not deployed)` : "") +
        `; run a full deploy from a checkout (nix run .#message-bridge-deploy), which uploads the Bitwarden ` +
        `token and makes it live`,
    );
  }
}

async function deploymentsNewestFirst() {
  const deployments = (await cfApi("/deployments"))?.deployments ?? [];
  return [...deployments].sort((a, b) => Date.parse(b.created_on) - Date.parse(a.created_on));
}

async function liveVersions() {
  return (await deploymentsNewestFirst())[0]?.versions ?? [];
}

async function workerExists() {
  const res = await fetch(`${api}/settings`, { headers: { authorization: `Bearer ${apiToken}` } });
  let body = null;
  try {
    body = await res.json();
  } catch {
    body = null;
  }
  return classifyWorkerProbe(res.status, body);
}

function stagePackage() {
  // A writable copy (the store path is read-only; cf and wrangler write
  // build state next to the config) with node_modules pointing at the
  // flake's cf and wrangler, which cf needs to resolve cf/config and to
  // delegate the build. deploy.nix sets both paths.
  const cfModule = process.env.MB_CF_MODULE;
  const wranglerModule = process.env.MB_WRANGLER_MODULE;
  if (!cfModule || !wranglerModule) {
    throw new Error("MB_CF_MODULE and MB_WRANGLER_MODULE are not set; run through nix run .#message-bridge-deploy");
  }
  const dir = mkdtempSync(join(tmpdir(), "message-bridge-deploy-"));
  cpSync(pkgDir, dir, { recursive: true, filter: (src) => !src.includes("/node_modules") });
  execFileSync("chmod", ["-R", "u+w", dir]);
  if (!existsSync(join(dir, "cloudflare.config.ts"))) {
    throw new Error(`no cloudflare.config.ts in the staged Worker package from ${pkgDir}`);
  }
  mkdirSync(join(dir, "node_modules"));
  symlinkSync(cfModule, join(dir, "node_modules", "cf"));
  symlinkSync(wranglerModule, join(dir, "node_modules", "wrangler"));
  chmodSync(dir, 0o700);
  return dir;
}

function runCf(cwd, cfArgs) {
  return execFileSync("cf", ["-q", ...cfArgs], { cwd, env, stdio: ["ignore", "inherit", "inherit"] });
}

async function cfApi(path) {
  const res = await fetch(`${api}${path}`, { headers: { authorization: `Bearer ${apiToken}` } });
  if (!res.ok) throw new Error(`Cloudflare API ${path} returned HTTP ${res.status}`);
  return (await res.json()).result;
}

// Every version, across pages (the API pages 10 at a time by default).
async function listVersions(params = {}) {
  const all = [];
  for (let page = 1; page <= 50; page++) {
    const qs = new URLSearchParams({ ...params, page: String(page), per_page: "100" });
    const res = await fetch(`${api}/versions?${qs}`, { headers: { authorization: `Bearer ${apiToken}` } });
    if (!res.ok) throw new Error(`Cloudflare API /versions returned HTTP ${res.status}`);
    const body = await res.json();
    const items = Array.isArray(body.result) ? body.result : (body.result?.items ?? []);
    all.push(...items);
    const total = body.result_info?.total_count;
    if (items.length === 0 || typeof total !== "number" || all.length >= total) return all;
  }
  throw new Error("Cloudflare API /versions kept paging past 50 pages");
}


// The version must be deployable and must carry ACCESS_TOKEN: without it
// the Worker answers 404 everywhere, which would take the connector down.
async function assertDeployable(id) {
  const deployable = (await listVersions({ deployable: "true" })).map((v) => v.id);
  if (!deployable.includes(id)) throw new Error(`version ${id} is not deployable; refusing to deploy it`);
  const detail = await cfApi(`/versions/${id}`);
  const hasToken = (detail?.resources?.bindings ?? []).some(
    (b) => b.name === "ACCESS_TOKEN" && b.type === "secret_text",
  );
  if (!hasToken) {
    throw new Error(`version ${id} has no ACCESS_TOKEN secret; refusing to deploy it (run a full deploy from a checkout)`);
  }
}

function createDeployment(cwd, id, { bypassChecks = false } = {}) {
  console.log(`deploying version ${id} at 100%${bypassChecks ? " (allowing a secret change)" : ""}`);
  runCf(cwd, [
    "workers",
    "deployments",
    "create",
    "--worker",
    WORKER,
    "--strategy",
    "percentage",
    "--versions",
    JSON.stringify([{ version_id: id, percentage: 100 }]),
    ...(bypassChecks ? ["--bypass-deployment-checks"] : []),
  ]);
}

// Checks the deployment API serves `id`, then waits for /healthz to answer
// from that version and checks its token. A code-only version must
// answer with the live token's fingerprint: whatever the versions list
// showed, this compares the token itself. On any failure after a
// promotion, promoteOrRollBack puts the previous live version (which had
// the right token) back at once and the deploy fails.
async function verifyLive(id, { expectFingerprint = null, requireVersion = false, replaced = null, own = null } = {}) {
  // The deployments list can lag a just-created deployment by a few seconds.
  let newest;
  for (let attempt = 1; attempt <= 10 && !isOwnDeployment(newest, id); attempt++) {
    if (attempt > 1) await new Promise((r) => setTimeout(r, 3000));
    newest = (await deploymentsNewestFirst())[0];
  }
  if (!isOwnDeployment(newest, id)) {
    throw new Error(`newest deployment does not serve version ${id} at 100%; check the dashboard`);
  }
  if (own) own.deploymentId = newest.id ?? null;
  // The token fingerprint was read from the version live before the
  // promote; it only describes what this replaced if no other deployment
  // went live in between.
  if (replaced) {
    const other = replacedOtherThan(await deploymentsNewestFirst(), id, replaced);
    if (other) {
      const err = new Error(
        `another deployment (version ${other}) went live between this deploy's checks and its promotion, ` +
          `so its token check no longer applies`,
      );
      err.rollbackTo = other;
      throw err;
    }
  }
  // An older version (a rollback target) may not report its version id;
  // then the configured check is all there is.
  let health = null;
  for (let attempt = 1; attempt <= 20; attempt++) {
    if (attempt > 1) await new Promise((r) => setTimeout(r, 3000));
    health = await readHealth();
    if (health?.version === id) break;
    if (!requireVersion && attempt >= 5 && health?.configured && health?.version == null) break;
  }
  const versionProblem = healthVersionProblem(health, id, { requireVersion });
  if (versionProblem) {
    throw new Error(`${publicUrl}/healthz ${versionProblem}; check the dashboard`);
  }
  if (!health?.ok || !health?.configured) {
    throw new Error(`${publicUrl}/healthz does not report an access token after deploying ${id}; roll back with --deploy-version`);
  }
  if (expectFingerprint && health.token_fingerprint !== expectFingerprint) {
    throw new Error(
      `version ${id} went live with a different ACCESS_TOKEN than expected; ` +
        `run a full deploy from a checkout (nix run .#message-bridge-deploy)`,
    );
  }
}

async function readHealth() {
  try {
    const res = await fetch(`${publicUrl}/healthz`, { cache: "no-store" });
    return res.ok ? await res.json() : null;
  } catch {
    return null;
  }
}
