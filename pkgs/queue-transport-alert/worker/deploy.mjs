// Deploys the queue-transport-alert Worker (code#6369) from its flake
// package: stages the package (or, run directly from a checkout, the
// checkout), runs `wrangler deploy` from the staged copy, then sets
// the Worker's three secrets with one `wrangler secret bulk` read from
// stdin, so no secret touches disk.
//
// Usage:
//   deploy.mjs [--dry-run]
//
// Needs CLOUDFLARE_ACCOUNT_ID and BWS_ACCESS_TOKEN; deploy.sh sets
// them. Every value comes from Bitwarden by name:
//   QTA_CF_TOKEN_SECRET       (cloudflare.api-token-day-to-day)   wrangler's API token
//   QTA_ACCESS_ID_SECRET      (cloudflare.access.queue-broker-client-id)
//   QTA_ACCESS_SECRET_SECRET  (cloudflare.access.queue-broker-client-secret)
//   QTA_GITHUB_TOKEN_SECRET   (github.developing-today-code-automation)
// No secret is ever printed.
import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const WORKER = "queue-transport-alert";

function usageError(message) {
  console.error(`${message}\nusage: deploy.mjs [--dry-run]`);
  process.exit(2);
}
let dryRun = false;
for (const arg of process.argv.slice(2)) {
  if (arg === "--dry-run") {
    if (dryRun) usageError("duplicate argument: --dry-run");
    dryRun = true;
  } else {
    usageError(`unrecognized argument: ${arg}`);
  }
}

if (!process.env.CLOUDFLARE_ACCOUNT_ID) throw new Error("CLOUDFLARE_ACCOUNT_ID is not set; run deploy.sh");
const names = {
  cf: process.env.QTA_CF_TOKEN_SECRET ?? "cloudflare.api-token-day-to-day",
  accessId: process.env.QTA_ACCESS_ID_SECRET ?? "cloudflare.access.queue-broker-client-id",
  accessSecret: process.env.QTA_ACCESS_SECRET_SECRET ?? "cloudflare.access.queue-broker-client-secret",
  github: process.env.QTA_GITHUB_TOKEN_SECRET ?? "github.developing-today-code-automation",
};
const vault = JSON.parse(execFileSync("bws", ["secret", "list", "-o", "json"], { encoding: "utf8" }));
const byName = (name) => {
  const value = vault.find((s) => s.key === name)?.value;
  if (!value) throw new Error(`no Bitwarden secret named ${name}`);
  return value.replace(/\r?\n$/, "");
};
const env = { ...process.env, CLOUDFLARE_API_TOKEN: byName(names.cf) };
const workerSecrets = {
  ACCESS_CLIENT_ID: byName(names.accessId),
  ACCESS_CLIENT_SECRET: byName(names.accessSecret),
  GITHUB_TOKEN: byName(names.github),
};

// Stage a writable copy (the store path is read-only; wrangler writes
// .wrangler state next to the config).
const pkgDir = process.env.QUEUE_TRANSPORT_ALERT_PKG ?? here;
const stage = mkdtempSync(join(tmpdir(), "queue-transport-alert-deploy-"));
cpSync(pkgDir, stage, { recursive: true });
if (!existsSync(join(stage, "wrangler.jsonc"))) {
  throw new Error(`no wrangler.jsonc in staged Worker package at ${pkgDir}`);
}

const wranglerArgs = ["deploy"];
if (dryRun) wranglerArgs.push("--dry-run");
execFileSync("wrangler", wranglerArgs, { cwd: stage, env, stdio: "inherit" });
if (dryRun) {
  console.log(`dry run: config builds, nothing uploaded; would set secrets ${Object.keys(workerSecrets).join(", ")}`);
  process.exit(0);
}
execFileSync("wrangler", ["secret", "bulk"], {
  cwd: stage,
  env,
  input: JSON.stringify(workerSecrets),
  stdio: ["pipe", "inherit", "inherit"],
});
console.log(`deployed: ${WORKER} (wrangler deploy from the flake package; secrets ${Object.keys(workerSecrets).join(", ")} set)`);
