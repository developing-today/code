// Deploys the message-bridge Worker from its flake package: stages the
// package (or, run directly from a checkout, the checkout), runs
// `wrangler deploy` from the staged copy, then sets the Worker's one
// secret, ACCESS_TOKEN, with `wrangler secret bulk` read from stdin, so
// the token never touches disk.
//
// Usage:
//   deploy.mjs [--dry-run | --print-urls]
//
// --print-urls prints the connector and page URLs, which contain the
// access token, and deploys nothing. Nothing else ever prints it.
//
// Needs BWS_ACCESS_TOKEN (and CLOUDFLARE_ACCOUNT_ID to deploy);
// deploy.sh sets them. Values come from Bitwarden by name:
//   MB_CF_TOKEN_SECRET      (cloudflare.api-token-day-to-day)   wrangler's API token
//   MB_ACCESS_TOKEN_SECRET  (message-bridge.access-token)       the path token
// The first deploy creates the access token (32 random bytes,
// base64url) in Bitwarden project MB_BWS_PROJECT (default: the
// "developing today" project) when it doesn't exist yet. To rotate it,
// delete or edit that secret and deploy again.
import { execFileSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import { cpSync, existsSync, mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const WORKER = "message-bridge";
const MIN_TOKEN_LENGTH = 32;

function usageError(message) {
  console.error(`${message}\nusage: deploy.mjs [--dry-run | --print-urls]`);
  process.exit(2);
}
const args = process.argv.slice(2);
if (args.length > 1) usageError("give at most one of --dry-run, --print-urls");
if (args.length === 1 && !["--dry-run", "--print-urls"].includes(args[0])) {
  usageError(`unrecognized argument: ${args[0]}`);
}
// "deploy" is only the internal name for the no-argument default.
const mode = args[0] ?? "deploy";

const names = {
  cf: process.env.MB_CF_TOKEN_SECRET ?? "cloudflare.api-token-day-to-day",
  access: process.env.MB_ACCESS_TOKEN_SECRET ?? "message-bridge.access-token",
};
const project = process.env.MB_BWS_PROJECT ?? "d370cd10-3d67-4b34-b61a-b4dc00753f97";

const vault = JSON.parse(execFileSync("bws", ["secret", "list", "-o", "json"], { encoding: "utf8" }));
const find = (name) => vault.find((s) => s.key === name)?.value?.replace(/\r?\n$/, "");
const byName = (name) => {
  const value = find(name);
  if (!value) throw new Error(`no Bitwarden secret named ${name}`);
  return value;
};

const pkgDir = process.env.MESSAGE_BRIDGE_PKG ?? here;
const wranglerConfig = readFileSync(join(pkgDir, "wrangler.jsonc"), "utf8");
const publicUrl = (wranglerConfig.match(/"PUBLIC_URL"\s*:\s*"([^"]+)"/)?.[1] ?? "").replace(/\/+$/, "");

let accessToken = find(names.access);
if (accessToken && accessToken.length < MIN_TOKEN_LENGTH) {
  throw new Error(`Bitwarden secret ${names.access} is shorter than ${MIN_TOKEN_LENGTH} characters; the Worker would refuse it`);
}

if (mode === "--print-urls") {
  if (!accessToken) throw new Error(`no Bitwarden secret named ${names.access}; deploy once to create it`);
  if (!publicUrl) throw new Error("no PUBLIC_URL in wrangler.jsonc");
  console.log(`connector: ${publicUrl}/mcp/${accessToken}`);
  console.log(`page:      ${publicUrl}/p/${accessToken}`);
  console.log(`api:       ${publicUrl}/api/${accessToken}/messages`);
  process.exit(0);
}

if (!process.env.CLOUDFLARE_ACCOUNT_ID) throw new Error("CLOUDFLARE_ACCOUNT_ID is not set; run deploy.sh");
const env = { ...process.env, CLOUDFLARE_API_TOKEN: byName(names.cf) };

// Stage a writable copy (the store path is read-only; wrangler writes
// .wrangler state next to the config).
const stage = mkdtempSync(join(tmpdir(), "message-bridge-deploy-"));
cpSync(pkgDir, stage, { recursive: true });
if (!existsSync(join(stage, "wrangler.jsonc"))) {
  throw new Error(`no wrangler.jsonc in staged Worker package at ${pkgDir}`);
}

const wranglerArgs = ["deploy"];
if (mode === "--dry-run") wranglerArgs.push("--dry-run");
execFileSync("wrangler", wranglerArgs, { cwd: stage, env, stdio: "inherit" });
if (mode === "--dry-run") {
  console.log(
    `dry run: config builds, nothing uploaded; would set ACCESS_TOKEN from ${names.access}` +
      (accessToken ? "" : " (creating it first)"),
  );
  process.exit(0);
}

if (!accessToken) {
  accessToken = randomBytes(32).toString("base64url");
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
      accessToken,
      project,
    ],
    { stdio: ["ignore", "ignore", "inherit"] },
  );
  console.log(`created Bitwarden secret ${names.access}`);
}

execFileSync("wrangler", ["secret", "bulk"], {
  cwd: stage,
  env,
  input: JSON.stringify({ ACCESS_TOKEN: accessToken }),
  stdio: ["pipe", "inherit", "inherit"],
});
console.log(`deployed: ${WORKER} (${publicUrl}); ACCESS_TOKEN set. Run with --print-urls for the connector URL.`);
