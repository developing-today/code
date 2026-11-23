// Build and push only the Lab Ops Hub crawler image. Worker versions are
// uploaded and promoted by the separate, explicitly dispatched GHA workflows.
// Keep this immutable image pin aligned with container-image.nix and
// wrangler.jsonc; changing it requires publishing the image before a Worker
// release can reference it.
//
// Usage: image-push.mjs [--dry-run]
//   --dry-run builds the Nix image but skips Cloudflare credentials and push.
//
// Needs CLOUDFLARE_ACCOUNT_ID and BWS_ACCESS_TOKEN; image-push.sh sets them.
// The account-wide push token comes from Bitwarden by name. It is passed to
// Wrangler only in the child environment; registry JWT is held in a temporary
// Docker config and is never passed in argv or printed.
import { execFileSync } from "node:child_process";
import {
  cpSync,
  existsSync,
  mkdtempSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const IMAGE = "lab-ops-hub-crawlercontainer:v5";

const args = process.argv.slice(2);
function usageError(message) {
  console.error(`${message}\nusage: image-push.mjs [--dry-run]`);
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
if (process.env.LOH_CHECK_ARGS_ONLY === "1") process.exit(0);

// The build path is intentionally shared with container-images.yml. The
// Worker release workflow never builds or pushes this image.
console.log("building crawler container image (container-image.nix)");
const imageTar = execFileSync(
  "nix",
  ["build", "-f", join(here, "container-image.nix"), "--no-link", "--print-out-paths"],
  { encoding: "utf8" },
).trim().split("\n").pop();
if (dryRun) {
  console.log("dry run: image built, nothing pushed; Worker version unchanged");
  process.exit(0);
}

requireEnv("CLOUDFLARE_ACCOUNT_ID");
const accountId = process.env.CLOUDFLARE_ACCOUNT_ID;
const secrets = JSON.parse(execFileSync("bws", ["secret", "list", "-o", "json"], { encoding: "utf8" }));
const byName = (name) => secrets.find((secret) => secret.key === name)?.value;
const tokenSecretName = process.env.LOH_CF_TOKEN_SECRET ?? "cloudflare.api-token-day-to-day";
const apiToken = byName(tokenSecretName);
if (!apiToken) throw new Error(`no Bitwarden secret named ${tokenSecretName}`);
const pushTokenName = process.env.LOH_CF_ACCOUNT_TOKEN_SECRET;
const pushToken = pushTokenName ? byName(pushTokenName) : apiToken;
if (pushTokenName && !pushToken) {
  throw new Error(`no Bitwarden secret named ${pushTokenName} (LOH_CF_ACCOUNT_TOKEN_SECRET)`);
}

const pkgDir = process.env.LAB_OPS_HUB_PKG ?? here;
const stage = mkdtempSync(join(tmpdir(), "lab-ops-hub-image-push-"));
const dockerConfig = mkdtempSync(join(tmpdir(), "lab-ops-hub-docker-config-"));
try {
  // Wrangler may create local state, so stage the read-only flake package.
  cpSync(pkgDir, stage, { recursive: true });
  if (!existsSync(join(stage, "wrangler.jsonc"))) {
    throw new Error(`no wrangler.jsonc in staged Worker package at ${pkgDir}`);
  }
  const wranglerBin = existsSync(join(stage, "node_modules", ".bin", "wrangler"))
    ? join(stage, "node_modules", ".bin", "wrangler")
    : "wrangler";

  let registryJwt;
  try {
    const out = execFileSync(wranglerBin, ["containers", "registries", "credentials", "--push"], {
      cwd: stage,
      env: { ...process.env, CLOUDFLARE_API_TOKEN: pushToken },
      encoding: "utf8",
    });
    registryJwt = out.trim().split(/\s+/).pop();
  } catch {
    throw new Error(
      "could not mint registry push credentials: `wrangler containers registries credentials --push` " +
      "requires the account-wide Cloudflare token (the day-to-day token gets Forbidden). Name the " +
      "Bitwarden secret holding it via LOH_CF_ACCOUNT_TOKEN_SECRET and retry.",
    );
  }

  // Push with crane, username = account id, password = JWT. The JWT never
  // reaches argv and the temporary Docker auth file is removed after push.
  const auth = Buffer.from(`${accountId}:${registryJwt}`).toString("base64");
  writeFileSync(
    join(dockerConfig, "config.json"),
    JSON.stringify({ auths: { "registry.cloudflare.com": { auth } } }),
    { mode: 0o600 },
  );
  const imageRef = `registry.cloudflare.com/${accountId}/${IMAGE}`;
  console.log(`pushing container image ${imageRef}`);
  execFileSync("crane", ["push", imageTar, imageRef], {
    env: { ...process.env, DOCKER_CONFIG: dockerConfig },
    stdio: "inherit",
  });
  console.log(`image published: ${imageRef}; Worker version unchanged`);
} finally {
  rmSync(dockerConfig, { recursive: true, force: true });
  rmSync(stage, { recursive: true, force: true });
}

function requireEnv(name) {
  if (!process.env[name]) throw new Error(`${name} is not set; run image-push.sh`);
}
