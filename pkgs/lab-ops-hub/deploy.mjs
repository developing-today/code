// Deploys the lab-ops-hub Worker AND its crawler container image as
// one command — issue #5767, phase 2 ("worker deploy + its container
// image push become one `deploy lab-ops-hub`"). Previously: build the
// Nix image by hand, mint registry credentials by hand, crane push by
// hand, then a raw `npm run deploy` in a checkout.
//
// The Worker tree is a flake package (package.nix, lockfile-pinned
// deps including the pinned wrangler 4.152.0); this script stages that
// package (or, run directly from a checkout, the checkout itself) and
// runs the package's own wrangler from the staged copy. No
// deploy-time `npm install`.
//
// Steps (full deploy):
//   1. nix build -f container-image.nix (the only image build path;
//      pinned nixpkgs inside that file) -> docker-archive tar.
//   2. Mint temporary registry push credentials with
//      `wrangler containers registries credentials --push`. This
//      needs the ACCOUNT-WIDE Cloudflare token — the day-to-day token
//      gets Forbidden (README, observed 2026-10-20). Name the
//      Bitwarden secret holding it with LOH_CF_ACCOUNT_TOKEN_SECRET;
//      without one, the day-to-day token is tried and the failure
//      message names the remedy.
//   3. crane push the tar to the registry image wrangler.jsonc
//      references (lab-ops-hub-crawlercontainer:v5 — keep in step
//      with container-image.nix's tag).
//   4. wrangler deploy from the staged package.
//
// Usage:
//   deploy.mjs [--dry-run]                 validate the Worker config
//                                          only (no image push, no
//                                          upload)
//
// Needs CLOUDFLARE_ACCOUNT_ID and BWS_ACCESS_TOKEN; deploy.sh sets
// them. API tokens come from Bitwarden by name (deploy token default
// cloudflare.api-token-day-to-day; override with LOH_CF_TOKEN_SECRET).
// No secret is ever printed; the registry JWT passes to crane through
// a temporary docker config directory, never argv or logs.
import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const WORKER = "lab-ops-hub";
const IMAGE = "lab-ops-hub-crawlercontainer:v5"; // container-image.nix tag

const args = process.argv.slice(2);
// Strict argument validation, following the sibling deploy scripts
// (#3287): the only accepted invocation is `deploy.mjs [--dry-run]`.
function usageError(message) {
  console.error(`${message}\nusage: deploy.mjs [--dry-run]`);
  process.exit(2);
}
let dryRun = false;
for (const arg of args) {
  if (arg === "--dry-run") {
    if (dryRun) usageError("duplicate argument: --dry-run");
    dryRun = true;
  } else {
    usageError(`unrecognized argument: ${arg}`);
  }
}

requireEnv("CLOUDFLARE_ACCOUNT_ID");
const accountId = process.env.CLOUDFLARE_ACCOUNT_ID;
const secrets = JSON.parse(execFileSync("bws", ["secret", "list", "-o", "json"], { encoding: "utf8" }));
const byName = (name) => secrets.find((secret) => secret.key === name)?.value;
const tokenSecretName = process.env.LOH_CF_TOKEN_SECRET ?? "cloudflare.api-token-day-to-day";
const apiToken = byName(tokenSecretName);
if (!apiToken) {
  throw new Error(`no Bitwarden secret named ${tokenSecretName}`);
}
const env = { ...process.env, CLOUDFLARE_API_TOKEN: apiToken };

// Stage a writable copy of the Worker package (the store path is
// read-only; wrangler writes .wrangler state next to the config).
const pkgDir = process.env.LAB_OPS_HUB_PKG ?? here;
const stage = mkdtempSync(join(tmpdir(), "lab-ops-hub-deploy-"));
cpSync(pkgDir, stage, { recursive: true });
if (!existsSync(join(stage, "wrangler.jsonc"))) {
  throw new Error(`no wrangler.jsonc in staged Worker package at ${pkgDir}`);
}
const wranglerBin = existsSync(join(stage, "node_modules", ".bin", "wrangler"))
  ? join(stage, "node_modules", ".bin", "wrangler")
  : "wrangler";

if (dryRun) {
  runWrangler(["deploy", "--dry-run"]);
  console.log("dry run: config builds, nothing uploaded, image not pushed");
  process.exit(0);
}

// 1. Build the crawler container image (daemon-less, dockerTools).
console.log("building crawler container image (container-image.nix)");
const imageTar = execFileSync(
  "nix",
  ["build", "-f", join(here, "container-image.nix"), "--no-link", "--print-out-paths"],
  { encoding: "utf8" }
).trim().split("\n").pop();

// 2. Mint temporary registry push credentials (account-wide token).
const pushTokenName = process.env.LOH_CF_ACCOUNT_TOKEN_SECRET;
const pushToken = pushTokenName ? byName(pushTokenName) : apiToken;
if (pushTokenName && !pushToken) {
  throw new Error(`no Bitwarden secret named ${pushTokenName} (LOH_CF_ACCOUNT_TOKEN_SECRET)`);
}
let registryJwt;
try {
  const out = execFileSync(wranglerBin, ["containers", "registries", "credentials", "--push"], {
    cwd: stage,
    env: { ...env, CLOUDFLARE_API_TOKEN: pushToken },
    encoding: "utf8",
  });
  registryJwt = out.trim().split(/\s+/).pop();
} catch {
  throw new Error(
    "could not mint registry push credentials: `wrangler containers registries credentials --push` " +
    "requires the ACCOUNT-WIDE Cloudflare token (the day-to-day token gets Forbidden — see " +
    "pkgs/lab-ops-hub/README.md). Name the Bitwarden secret holding it via " +
    "LOH_CF_ACCOUNT_TOKEN_SECRET and retry."
  );
}

// 3. Push the image with crane, username = account id, password = JWT
// (via a temporary docker config dir; the JWT never reaches argv).
const dockerConfig = mkdtempSync(join(tmpdir(), "lab-ops-hub-docker-config-"));
const auth = Buffer.from(`${accountId}:${registryJwt}`).toString("base64");
writeFileSync(
  join(dockerConfig, "config.json"),
  JSON.stringify({ auths: { "registry.cloudflare.com": { auth } } })
);
const imageRef = `registry.cloudflare.com/${accountId}/${IMAGE}`;
console.log(`pushing container image ${imageRef}`);
execFileSync("crane", ["push", imageTar, imageRef], {
  env: { ...process.env, DOCKER_CONFIG: dockerConfig },
  stdio: "inherit",
});

// 4. Deploy the Worker from the staged flake package, stamping the
// deployed commit as the VERSION var (#15793) when it is known:
// LOH_VERSION from the caller, else the HEAD of the checkout deploy.mjs
// runs in. The staged flake package carries no .git, so a nix deploy
// without LOH_VERSION deploys unstamped and /version answers
// "unknown", which the edge probe records honestly.
let versionSha = process.env.LOH_VERSION ?? "";
if (!/^[0-9a-f]{40}$/.test(versionSha)) {
  versionSha = "";
  try {
    const head = execFileSync("git", ["-C", here, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
    if (/^[0-9a-f]{40}$/.test(head)) versionSha = head;
  } catch {
    // Not a checkout: deploy unstamped.
  }
}
runWrangler(versionSha ? ["deploy", "--var", `VERSION:${versionSha}`] : ["deploy"]);
console.log(`deployed: ${WORKER} (image ${imageRef} pushed, wrangler deploy from the flake package)`);

function runWrangler(wranglerArgs) {
  execFileSync(wranglerBin, wranglerArgs, { cwd: stage, env, stdio: "inherit" });
}

function requireEnv(name) {
  if (!process.env[name]) throw new Error(`${name} is not set; run deploy.sh`);
}
