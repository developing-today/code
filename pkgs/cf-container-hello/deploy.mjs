// Deploys the cf-container-hello Worker — issue #5767, phase 2, with
// the status/rollback lifecycle verbs of the slice-4 Cloudflare
// lifecycle (the deploy CLI's provider boundary drives these verbs;
// see pkgs/deploy/deploy.py).
//
// Previously the only deploy path was a raw `npm run deploy` in a
// checkout (with floating deps: no lockfile existed). The Worker tree
// is now a flake package (package.nix, lockfile-pinned deps); this
// script stages that package (or, run directly from a checkout, the
// checkout itself) and runs wrangler deploy from the staged copy.
// The containers in wrangler.jsonc reference pre-pushed registry
// images, so no Docker daemon is involved.
//
// Usage:
//   deploy.mjs [--dry-run] [--json]             deploy (validate only with --dry-run)
//   deploy.mjs status [--json]                  what Cloudflare is serving now
//   deploy.mjs verify-bindings --version ID [--json]
//                                               binding names a version carries
//   deploy.mjs rollback --version ID [--json]   re-activate a recorded version
//
// With --json the script prints a single JSON object on stdout (the
// deploy CLI's provider boundary parses it); otherwise it prints
// human-readable lines. Needs CLOUDFLARE_ACCOUNT_ID and
// BWS_ACCESS_TOKEN; deploy.sh sets them. The API token comes from
// Bitwarden by name (default cloudflare.api-token-day-to-day;
// override with CFH_CF_TOKEN_SECRET). No secret is ever printed or
// written to disk.
import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const WORKER = "cf-container-hello";

const USAGE = `usage: deploy.mjs [--dry-run] [--json]
       deploy.mjs status [--json]
       deploy.mjs verify-bindings --version <version-id> [--json]
       deploy.mjs rollback --version <version-id> [--json]`;

function usageError(message) {
  console.error(`${message}\n${USAGE}`);
  process.exit(2);
}

// Strict argument validation, following the sibling deploy scripts
// (#3287): only the invocations in USAGE are accepted.
const rawArgs = process.argv.slice(2);
let verb = "deploy";
let dryRun = false;
let json = false;
let versionId = null;
const positional = [];
for (let i = 0; i < rawArgs.length; i++) {
  const arg = rawArgs[i];
  if (arg === "--dry-run") {
    if (dryRun) usageError("duplicate argument: --dry-run");
    dryRun = true;
  } else if (arg === "--json") {
    if (json) usageError("duplicate argument: --json");
    json = true;
  } else if (arg === "--version") {
    if (versionId !== null) usageError("duplicate argument: --version");
    versionId = rawArgs[++i];
    if (!versionId) usageError("--version needs a version id");
  } else if (arg.startsWith("--version=")) {
    if (versionId !== null) usageError("duplicate argument: --version");
    versionId = arg.slice("--version=".length);
    if (!versionId) usageError("--version needs a version id");
  } else if (arg.startsWith("-")) {
    usageError(`unrecognized argument: ${arg}`);
  } else {
    positional.push(arg);
  }
}
if (positional.length > 1) usageError(`unrecognized arguments: ${positional.join(" ")}`);
if (positional.length === 1) {
  verb = positional[0];
  if (!["status", "verify-bindings", "rollback"].includes(verb)) {
    usageError(`unrecognized verb: ${verb}`);
  }
}
if (verb === "deploy") {
  if (versionId !== null) usageError("--version is only for verify-bindings and rollback");
} else if (verb === "status") {
  if (versionId !== null) usageError("--version is only for verify-bindings and rollback");
  if (dryRun) usageError("--dry-run is only for deploy");
} else {
  // verify-bindings, rollback
  if (versionId === null) usageError(`${verb} needs --version <version-id>`);
  if (dryRun) usageError("--dry-run is only for deploy");
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

function runWrangler(args, { captureJson = false } = {}) {
  if (captureJson) {
    const out = execFileSync("wrangler", args, { cwd: stage, env, encoding: "utf8" });
    try {
      return JSON.parse(out);
    } catch (err) {
      throw new Error(`wrangler ${args.join(" ")} returned invalid JSON: ${err}\n${out}`);
    }
  }
  execFileSync("wrangler", args, { cwd: stage, env, stdio: "inherit" });
  return null;
}

// `wrangler deployments status --json` prints the raw latest-deployment
// object: { id, versions: [{ version_id, percentage }], ... }. The
// serving version is the 100% one (a plain deploy/rollback always
// leaves exactly one version at 100%).
function readServing() {
  const deployment = runWrangler(["deployments", "status", "--json"], { captureJson: true });
  const deploymentId = deployment?.id ?? deployment?.deployment_id ?? null;
  const versions = Array.isArray(deployment?.versions) ? deployment.versions : [];
  const serving =
    versions.find((v) => v?.percentage === 100) ?? versions.find((v) => v?.percentage === "100") ?? versions[0] ?? null;
  const versionIdFound = serving?.version_id ?? serving?.versionId ?? serving?.id ?? null;
  if (!versionIdFound || !deploymentId) {
    throw new Error(
      `could not determine the serving version/deployment from wrangler output: ${JSON.stringify(deployment).slice(0, 500)}`,
    );
  }
  return { version_id: versionIdFound, deployment_id: deploymentId };
}

// `wrangler versions view <id> --json` prints the raw version object,
// whose `resources.bindings` array carries { type, name, ... } per
// binding (the Cloudflare versions API shape).
function readVersionBindings(id) {
  const version = runWrangler(["versions", "view", id, "--json"], { captureJson: true });
  const resources = version?.resources ?? version?.bindings ?? null;
  const list = Array.isArray(resources) ? resources : (resources?.bindings ?? null);
  if (!Array.isArray(list)) {
    throw new Error(
      `could not determine the bindings of version ${id} from wrangler output: ${JSON.stringify(version).slice(0, 500)}`,
    );
  }
  const names = list.map((b) => b?.name).filter((n) => typeof n === "string" && n);
  return { version_id: version?.id ?? id, bindings: names };
}

function emit(obj, human) {
  if (json) {
    console.log(JSON.stringify(obj));
  } else {
    console.log(human);
  }
}

if (verb === "status") {
  const serving = readServing();
  emit(serving, `serving: cloudflare version ${serving.version_id} deployment ${serving.deployment_id}`);
} else if (verb === "verify-bindings") {
  const found = readVersionBindings(versionId);
  emit(found, `version ${found.version_id} bindings: ${found.bindings.join(", ") || "(none)"}`);
} else if (verb === "rollback") {
  // Non-interactive: the deploy CLI already verified bindings and the
  // operator already chose; message/yes skip wrangler's prompts. The
  // new deployment id is read back from what Cloudflare serves.
  runWrangler(["rollback", versionId, "-m", `deploy CLI rollback to ${versionId}`, "-y"]);
  const serving = readServing();
  emit(
    { version_id: versionId, deployment_id: serving.deployment_id },
    `rolled back: ${WORKER} now serves version ${versionId} (deployment ${serving.deployment_id}) — no rebuild, no re-upload`,
  );
} else {
  const wranglerArgs = ["deploy"];
  if (dryRun) wranglerArgs.push("--dry-run");
  runWrangler(wranglerArgs);
  if (dryRun) {
    console.log("dry run: config builds, nothing uploaded");
  } else if (json) {
    const serving = readServing();
    console.log(JSON.stringify(serving));
  } else {
    console.log(`deployed: ${WORKER} (wrangler deploy from the flake package)`);
  }
}

function requireEnv(name) {
  if (!process.env[name]) throw new Error(`${name} is not set; run deploy.sh`);
}
