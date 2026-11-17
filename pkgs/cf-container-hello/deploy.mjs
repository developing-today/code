// Deploys the cf-container-hello Worker — issue #5767, phase 2.
// Previously the only deploy path was a raw `npm run deploy` in a
// checkout (with floating deps: no lockfile existed). The Worker tree
// is now a flake package (package.nix, lockfile-pinned deps); this
// script stages that package (or, run directly from a checkout, the
// checkout itself) and runs wrangler deploy from the staged copy.
// The containers in wrangler.jsonc reference pre-pushed registry
// images, so no Docker daemon is involved.
//
// Usage:
//   deploy.mjs [--dry-run]                 validate, or deploy
//
// Needs CLOUDFLARE_ACCOUNT_ID and BWS_ACCESS_TOKEN; deploy.sh sets
// them. The API token comes from Bitwarden by name (default
// cloudflare.api-token-day-to-day; override with CFH_CF_TOKEN_SECRET).
// No secret is ever printed or written to disk.
import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const WORKER = "cf-container-hello";

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
const tokenSecretName = process.env.CFH_CF_TOKEN_SECRET ?? "cloudflare.api-token-day-to-day";
const secrets = JSON.parse(execFileSync("bws", ["secret", "list", "-o", "json"], { encoding: "utf8" }));
const byName = (name) => secrets.find((secret) => secret.key === name)?.value;
const apiToken = byName(tokenSecretName);
if (!apiToken) {
  throw new Error(`no Bitwarden secret named ${tokenSecretName}`);
}
const env = { ...process.env, CLOUDFLARE_API_TOKEN: apiToken };

// Stage a writable copy of the Worker package (the store path is
// read-only; wrangler writes .wrangler state next to the config).
const pkgDir = process.env.CF_CONTAINER_HELLO_PKG ?? here;
const stage = mkdtempSync(join(tmpdir(), "cf-container-hello-deploy-"));
cpSync(pkgDir, stage, { recursive: true });
if (!existsSync(join(stage, "wrangler.jsonc"))) {
  throw new Error(`no wrangler.jsonc in staged Worker package at ${pkgDir}`);
}

const wranglerArgs = ["deploy"];
if (dryRun) wranglerArgs.push("--dry-run");
execFileSync("wrangler", wranglerArgs, { cwd: stage, env, stdio: "inherit" });
console.log(
  dryRun
    ? "dry run: config builds, nothing uploaded"
    : `deployed: ${WORKER} (wrangler deploy from the flake package)`
);

function requireEnv(name) {
  if (!process.env[name]) throw new Error(`${name} is not set; run deploy.sh`);
}
