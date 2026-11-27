// Deploys the cf-container-hello Worker — issue #5767, phase 2, plus
// the generic deploy CLI's Cloudflare lifecycle verbs (#5767).
// Previously the only deploy path was a raw `npm run deploy` in a
// checkout (with floating deps: no lockfile existed). The Worker tree
// is now a flake package (package.nix, lockfile-pinned deps); this
// script stages that package (or, run directly from a checkout, the
// checkout itself) and runs wrangler deploy from the staged copy.
// The containers in wrangler.jsonc reference pre-pushed registry
// images, so no Docker daemon is involved.
//
// Usage (the whole command line, parsed strictly per #3352):
//   deploy.mjs [--dry-run]          standalone deploy (human output)
//   deploy.mjs deploy               publish; last stdout line is JSON
//   deploy.mjs status               serving version/deployment, JSON
//   deploy.mjs activate <version-id>
//                                   create a deployment of an existing
//                                   version at 100% (rollback: no
//                                   upload), JSON
//   deploy.mjs version-bindings <version-id>
//                                   the version's secret binding
//                                   names, JSON
//
// The verb forms are the provider boundary of the generic deploy CLI
// (pkgs/deploy/deploy.py): each answers exactly one JSON object on
// its last stdout line, {"ok": true, ...} or {"ok": false, "error"}.
// The CLI verifies a rollback target's bindings (version-bindings)
// before it ever asks for activate.
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

const USAGE =
  "usage: deploy.mjs [--dry-run] | deploy | status | activate <version-id> | version-bindings <version-id>";
function usageError(message) {
  console.error(`${message}\n${USAGE}`);
  process.exit(2);
}

const args = process.argv.slice(2);
let verb = "deploy";
let standalone = true;
let dryRun = false;
let versionId = null;
if (args.length === 0) {
  // standalone deploy
} else if (args.length === 1 && args[0] === "--dry-run") {
  dryRun = true;
} else if (args.length === 1 && args[0] === "deploy") {
  standalone = false;
} else if (args.length === 1 && args[0] === "status") {
  verb = "status";
  standalone = false;
} else if (args.length === 2 && (args[0] === "activate" || args[0] === "version-bindings")) {
  verb = args[0];
  standalone = false;
  versionId = args[1];
  if (!versionId || versionId.startsWith("--")) {
    usageError(`${verb} needs a version id`);
  }
} else {
  usageError(`unrecognized arguments: ${args.join(" ")}`);
}

function fail(error) {
  if (standalone) {
    console.error(error.message);
  } else {
    console.log(JSON.stringify({ ok: false, error: error.message }));
  }
  process.exit(1);
}

try {
  await run();
} catch (error) {
  fail(error);
}

async function run() {
  requireEnv("CLOUDFLARE_ACCOUNT_ID");
  const accountId = process.env.CLOUDFLARE_ACCOUNT_ID;
  const tokenSecretName = process.env.CFH_CF_TOKEN_SECRET ?? "cloudflare.api-token-day-to-day";
  const secrets = JSON.parse(execFileSync("bws", ["secret", "list", "-o", "json"], { encoding: "utf8" }));
  const byName = (name) => secrets.find((secret) => secret.key === name)?.value;
  const apiToken = byName(tokenSecretName);
  if (!apiToken) {
    throw new Error(`no Bitwarden secret named ${tokenSecretName}`);
  }
  const env = { ...process.env, CLOUDFLARE_API_TOKEN: apiToken };

  async function cfApi(path, { method = "GET", body } = {}) {
    const res = await fetch(
      `https://api.cloudflare.com/client/v4/accounts/${accountId}/workers/scripts/${WORKER}${path}`,
      {
        method,
        headers: {
          authorization: `Bearer ${apiToken}`,
          ...(body ? { "content-type": "application/json" } : {}),
        },
        body: body ? JSON.stringify(body) : undefined,
      },
    );
    const parsed = await res.json().catch(() => null);
    if (!res.ok || parsed?.success === false) {
      const detail = parsed?.errors?.map((e) => e.message).join("; ") || `HTTP ${res.status}`;
      throw new Error(`Cloudflare API ${method} ${path} failed: ${detail}`);
    }
    return parsed.result;
  }

  function deploymentItems(result) {
    if (Array.isArray(result)) return result;
    if (Array.isArray(result?.items)) return result.items;
    throw new Error("unexpected Cloudflare deployments response shape");
  }

  function deploymentTime(deployment) {
    const raw = deployment?.created_on ?? deployment?.metadata?.created_on ?? 0;
    const parsed = typeof raw === "number" ? raw : Date.parse(raw);
    return Number.isFinite(parsed) ? parsed : 0;
  }

  // What Cloudflare says is serving: the newest deployment's active
  // version. This is the record of authority for `deploy status` —
  // never a local log.
  async function serving() {
    const deployments = deploymentItems(await cfApi("/deployments"));
    const latest = [...deployments].sort((a, b) => deploymentTime(b) - deploymentTime(a))[0];
    if (!latest) throw new Error(`worker ${WORKER} has no deployments`);
    const active = (latest.versions ?? [])
      .filter((version) => Number(version.percentage) > 0)
      .sort((a, b) => Number(b.percentage) - Number(a.percentage));
    if (!active.length) throw new Error(`latest deployment ${latest.id} serves no version`);
    return { version_id: active[0].version_id, deployment_id: latest.id };
  }

  if (verb === "status") {
    console.log(JSON.stringify({ ok: true, ...(await serving()) }));
    return;
  }

  if (verb === "version-bindings") {
    const detail = await cfApi(`/versions/${encodeURIComponent(versionId)}`);
    const bindings = (detail?.resources?.bindings ?? [])
      .filter((binding) => binding.type === "secret_text")
      .map((binding) => binding.name);
    console.log(JSON.stringify({ ok: true, secret_bindings: bindings }));
    return;
  }

  if (verb === "activate") {
    // Rollback/deploy of an existing version: a new deployment at
    // 100%, no upload. The caller (the generic deploy CLI) has
    // already verified the version's secret bindings.
    const deployment = await cfApi("/deployments", {
      method: "POST",
      body: { versions: [{ version_id: versionId, percentage: 100 }] },
    });
    if (!deployment?.id) throw new Error("Cloudflare created no deployment");
    console.log(JSON.stringify({ ok: true, deployment_id: deployment.id }));
    return;
  }

  // verb === "deploy": stage a writable copy of the Worker package
  // (the store path is read-only; wrangler writes .wrangler state
  // next to the config) and publish from it.
  const pkgDir = process.env.CF_CONTAINER_HELLO_PKG ?? here;
  const stage = mkdtempSync(join(tmpdir(), "cf-container-hello-deploy-"));
  cpSync(pkgDir, stage, { recursive: true });
  if (!existsSync(join(stage, "wrangler.jsonc"))) {
    throw new Error(`no wrangler.jsonc in staged Worker package at ${pkgDir}`);
  }
  const wranglerArgs = ["deploy"];
  if (dryRun) wranglerArgs.push("--dry-run");
  if (standalone) {
    execFileSync("wrangler", wranglerArgs, { cwd: stage, env, stdio: "inherit" });
    console.log(
      dryRun
        ? "dry run: config builds, nothing uploaded"
        : `deployed: ${WORKER} (wrangler deploy from the flake package)`,
    );
    return;
  }
  // App mode: wrangler's progress goes to stderr so stdout stays a
  // clean channel for the JSON answer.
  const out = execFileSync("wrangler", wranglerArgs, {
    cwd: stage,
    env,
    encoding: "utf8",
    stdio: ["inherit", "pipe", "inherit"],
  });
  if (out) process.stderr.write(out);
  console.log(JSON.stringify({ ok: true, ...(await serving()) }));
}

function requireEnv(name) {
  if (!process.env[name]) throw new Error(`${name} is not set; run deploy.sh`);
}
