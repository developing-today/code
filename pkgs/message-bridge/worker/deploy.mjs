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
//   deploy.mjs --deploy-version <id> [--dry-run]
//                                     deploy (roll back to) an existing
//                                     version at 100%; --dry-run previews
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
  createDeployment(stage, id);
  await verifyLive(id);
  console.log(`live: ${publicUrl} at version ${id}`);
  process.exit(0);
}

const stage = stagePackage();
const sha = process.env.GITHUB_SHA || execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const tag = `git-${sha.slice(0, 12)}`;
const via = args.mode === "code-only" ? "message-bridge-deploy workflow" : "nix run .#message-bridge-deploy";
const createArgs = ["workers", "versions", "create", "--tag", tag, "--message", `deploy ${sha} via ${via}`];

if (args.dryRun) {
  runCf(stage, [...createArgs, "--dry-run"]);
  console.log("dry run: config builds, nothing uploaded");
  process.exit(0);
}

let secretsDir = null;
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
  createArgs.push("--secrets-file", secretsFile);
}

console.log(`uploading version (tag ${tag})`);
const before = Date.now();
try {
  runCf(stage, createArgs);
} finally {
  if (secretsDir) rmSync(secretsDir, { recursive: true, force: true });
}

// Re-uploading an unchanged tree makes a second version with the same
// tag, so take the newest one with this tag.
const version = (await listVersions())
  .filter((v) => v.annotations?.["workers/tag"] === tag)
  .sort((a, b) => createdOn(b) - createdOn(a))[0];
if (!version || createdOn(version) < before - 120_000) {
  throw new Error(`uploaded version with tag ${tag} not found in the versions list`);
}
console.log(`uploaded version ${version.id}`);
await assertDeployable(version.id);
createDeployment(stage, version.id);
await verifyLive(version.id);
console.log(`live: ${publicUrl} at version ${version.id} (${tag})`);
console.log(`roll back with: nix run .#message-bridge-deploy -- --deploy-version <older version id>`);

// --- helpers ---

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

async function listVersions(query = "") {
  const result = await cfApi(`/versions${query}`);
  return Array.isArray(result) ? result : (result?.items ?? []);
}

function createdOn(v) {
  const raw = v.metadata?.created_on ?? v.created_on ?? 0;
  const t = typeof raw === "number" ? raw : Date.parse(raw);
  return Number.isFinite(t) ? t : 0;
}

// The version must be deployable and must carry ACCESS_TOKEN: without it
// the Worker answers 404 everywhere, which would take the connector down.
async function assertDeployable(id) {
  const deployable = (await listVersions("?deployable=true")).map((v) => v.id);
  if (!deployable.includes(id)) throw new Error(`version ${id} is not deployable; refusing to deploy it`);
  const detail = await cfApi(`/versions/${id}`);
  const hasToken = (detail?.resources?.bindings ?? []).some(
    (b) => b.name === "ACCESS_TOKEN" && b.type === "secret_text",
  );
  if (!hasToken) {
    throw new Error(`version ${id} has no ACCESS_TOKEN secret; refusing to deploy it (run a full deploy from a checkout)`);
  }
}

function createDeployment(cwd, id) {
  console.log(`deploying version ${id} at 100%`);
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
  ]);
}

async function verifyLive(id) {
  const deployments = (await cfApi("/deployments"))?.deployments ?? [];
  const newest = [...deployments].sort((a, b) => Date.parse(b.created_on) - Date.parse(a.created_on))[0];
  const serving = newest?.versions ?? [];
  if (!(serving.length === 1 && serving[0].version_id === id && serving[0].percentage === 100)) {
    throw new Error(`newest deployment does not serve version ${id} at 100%; check the dashboard`);
  }
  let health = null;
  for (let attempt = 1; attempt <= 5 && !health?.configured; attempt++) {
    if (attempt > 1) await new Promise((r) => setTimeout(r, 3000));
    try {
      health = await (await fetch(`${publicUrl}/healthz`, { cache: "no-store" })).json();
    } catch {
      health = null;
    }
  }
  if (!health?.ok || !health?.configured) {
    throw new Error(`${publicUrl}/healthz does not report an access token after deploying ${id}; roll back with --deploy-version`);
  }
}
