// Validates config.json and uploads a Cloudflare Workers Version for the
// mcpx Worker. Uploading does not promote it, change the
// custom domain, or retire another Worker; promotion is a separate,
// verified action (promote.mjs). Modeled on the id-mail publisher
// (nixos/services/id-mail/worker/upload-version.mjs), minus the mail
// config surface and with a first-publish path: a Worker with no
// deployment yet records a null baseline instead of refusing.
// Usage: upload-version.mjs [--dry-run]. Uploads require the dedicated
// mcpx-production environment token and a secrets file
// holding the bridge-door Access credentials.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const cfBin = join(here, "node_modules", ".bin", "cf");
const WORKFLOW = ".github/workflows/mcpx-worker-publish.yml";

// Strict argument validation: the only accepted invocation is
// `upload-version.mjs [--dry-run]`. Anything else aborts before any
// config, cf or API work.
const args = process.argv.slice(2);
const unknownArgs = args.filter((arg) => arg !== "--dry-run");
if (args.length > 1 || unknownArgs.length > 0) {
  const message =
    unknownArgs.length > 0
      ? `unrecognized argument(s): ${unknownArgs.join(" ")}`
      : `duplicate argument(s): ${[...new Set(args)].join(" ")}`;
  console.error(`${message}\nusage: upload-version.mjs [--dry-run]`);
  process.exit(2);
}
const dryRun = args.includes("--dry-run");

const config = JSON.parse(readFileSync(join(here, "config.json"), "utf8"));
const problems = validate(config);
if (problems.length > 0) {
  throw new Error(`config.json is invalid:\n  ${problems.join("\n  ")}`);
}

requireEnv("CLOUDFLARE_ACCOUNT_ID");
const uploadToken = dryRun ? undefined : requireEnv("MCPX_WORKER_CF_TOKEN");
const secretsFile = dryRun ? undefined : requireEnv("MCPX_WORKER_SECRETS_FILE");
if (!dryRun && !existsSync(secretsFile)) {
  throw new Error("MCPX_WORKER_SECRETS_FILE does not exist; refusing to upload a version without its bridge credentials");
}

// The locked toolchain is installed in a separate credential-free
// workflow step; an authenticated run never spawns a package installer.
if (!existsSync(cfBin)) {
  throw new Error("pinned cf CLI missing; install dependencies before supplying credentials");
}

const sha = process.env.GITHUB_SHA ?? execFileSync("git", ["rev-parse", "HEAD"], { cwd: here, encoding: "utf8" }).trim();
const runId = process.env.GITHUB_RUN_ID;
const runAttempt = process.env.GITHUB_RUN_ATTEMPT;
const provenancePath = dryRun ? undefined : requireEnv("MCPX_WORKER_PROVENANCE_FILE");
if (!dryRun) {
  if (!/^\d+$/.test(runId ?? "") || !/^\d+$/.test(runAttempt ?? "")) {
    throw new Error("GITHUB_RUN_ID and GITHUB_RUN_ATTEMPT are required for provenance");
  }
  if (process.env.GITHUB_EVENT_NAME !== "workflow_dispatch" || process.env.GITHUB_REF !== "refs/heads/main") {
    throw new Error("uploads are allowed only from a manual workflow run on main");
  }
  if (!/^[0-9a-f]{40}$/.test(sha) || !process.env.GITHUB_REPOSITORY) {
    throw new Error("a full commit SHA and repository identity are required for provenance");
  }
}
const tag = dryRun ? `git-${sha.slice(0, 12)}` : `mcpx-${sha.slice(0, 12)}-run-${runId}-attempt-${runAttempt}`;
const cfArgs = [
  "workers", "versions", "create",
  "--worker", config.worker,
  "--tag", tag,
  "--message", `publish ${sha} for ${config.worker} (upload only)`,
];
if (secretsFile) cfArgs.push("--secrets-file", secretsFile);
if (dryRun) cfArgs.push("--dry-run");

function cfJson(argv) {
  const output = execFileSync(cfBin, argv, {
    cwd: here,
    encoding: "utf8",
    env: uploadToken ? { ...process.env, CLOUDFLARE_API_TOKEN: uploadToken } : process.env,
  });
  return unwrapJson(output);
}

// First publish: the Worker does not exist yet, and cf answers the
// deployments list with APIError 10007 (404) instead of an empty list.
// That specific shape means "no baseline"; every other failure is a
// real error and keeps propagating.
function listDeployments() {
  try {
    return cfJson(["workers", "deployments", "list", "--worker", config.worker, "--per-page", "1"]);
  } catch (err) {
    const text = `${err.stdout ?? ""}\n${err.stderr ?? ""}\n${err.message ?? ""}`;
    if (text.includes("10007") || text.includes("does not exist on your account")) {
      return null;
    }
    throw err;
  }
}

let baselineDeployment = null;
if (!dryRun) {
  const deployments = listDeployments();
  baselineDeployment = deployments ? firstDeployment(deployments) : null;
  if (baselineDeployment) {
    if (typeof baselineDeployment?.versions?.[0]?.version_id !== "string" || !baselineDeployment.id) {
      throw new Error("cf could not identify the current active Worker deployment; refusing to upload");
    }
    const versions = cfJson(["workers", "versions", "list", "--worker-id", config.worker, "--per-page", "1"]);
    const latestVersion = firstVersion(versions);
    if (latestVersion?.id !== baselineDeployment.versions[0].version_id) {
      throw new Error("latest uploaded version is not the current active version; refusing to inherit secrets from an unpromoted version");
    }
  }
}

const output = execFileSync(cfBin, cfArgs, {
  cwd: here,
  encoding: "utf8",
  env: uploadToken ? { ...process.env, CLOUDFLARE_API_TOKEN: uploadToken } : process.env,
});
if (dryRun) {
  process.stdout.write(output);
  console.log(`dry run: ${config.worker} version is valid; nothing uploaded`);
} else {
  // cf's versions-create stdout is a human panel in this build, not
  // JSON — resolve the uploaded version by its unique tag instead of
  // parsing that output. Exactly one version may carry the tag.
  const listed = cfJson(["workers", "versions", "list", "--worker-id", config.worker, "--per-page", "50"]);
  const allVersions = Array.isArray(listed) ? listed : listed?.versions ?? listed?.items ?? [];
  const matches = allVersions.filter((v) => v.annotations?.["workers/tag"] === tag);
  if (matches.length !== 1) {
    throw new Error(`expected exactly one uploaded version with tag ${tag}, found ${matches.length}; refusing to emit provenance`);
  }
  const version = matches[0];
  const versionId = version.id ?? version.version_id;
  if (typeof versionId !== "string" || !versionId) {
    throw new Error("cf did not return the uploaded version ID; refusing to emit provenance");
  }
  if (baselineDeployment) {
    const deployment = firstDeployment(cfJson(["workers", "deployments", "list", "--worker", config.worker, "--per-page", "1"]));
    if (deployment?.id !== baselineDeployment.id || deployment?.versions?.[0]?.version_id !== baselineDeployment.versions[0].version_id) {
      throw new Error("the active deployment changed during upload; refusing to emit provenance");
    }
  }
  const provenance = {
    schema: 1,
    repository: process.env.GITHUB_REPOSITORY,
    workflow: WORKFLOW,
    event: process.env.GITHUB_EVENT_NAME,
    ref: process.env.GITHUB_REF,
    sha,
    runId,
    runAttempt,
    worker: config.worker,
    tag,
    versionId,
    baselineDeploymentId: baselineDeployment?.id ?? null,
    baselineVersionId: baselineDeployment?.versions?.[0]?.version_id ?? null,
  };
  if (provenance.event !== "workflow_dispatch" || provenance.ref !== "refs/heads/main") {
    throw new Error("upload provenance must come from a manual run on main");
  }
  writeFileSync(provenancePath, `${JSON.stringify(provenance, null, 2)}\n`, { mode: 0o600 });
  console.log(`uploaded ${config.worker} version ${versionId} with tag ${tag}; promotion is separate`);
}

function requireEnv(name) {
  const value = process.env[name];
  if (!value) {
    throw new Error(`${name} is not set`);
  }
  return value;
}

function unwrapJson(output) {
  let parsed;
  try {
    parsed = JSON.parse(output);
  } catch {
    throw new Error("cf returned non-JSON output; refusing to continue");
  }
  return parsed?.result ?? parsed;
}

function firstDeployment(result) {
  if (Array.isArray(result)) return result[0];
  if (Array.isArray(result?.deployments)) return result.deployments[0];
  return undefined;
}

function firstVersion(result) {
  if (Array.isArray(result)) return result[0];
  if (Array.isArray(result?.versions)) return result.versions[0];
  if (Array.isArray(result?.items)) return result.items[0];
  return undefined;
}

function validate(c) {
  const errors = [];
  if (c === null || typeof c !== "object" || Array.isArray(c)) {
    errors.push("config must be a JSON object");
    return errors;
  }
  const check = (ok, message) => {
    if (!ok) errors.push(message);
  };
  check(/^[a-z0-9-]+$/.test(c.worker ?? ""), "worker must be a lowercase name");
  check(/^\d{4}-\d{2}-\d{2}$/.test(c.compatibilityDate ?? ""), "compatibilityDate must be YYYY-MM-DD");
  check(typeof c.workersDev === "boolean", "workersDev must be true or false");
  check(
    Array.isArray(c.routes) && c.routes.length > 0 && c.routes.every((r) => typeof r === "string" && r.includes(".")),
    "routes must be a non-empty list of hostnames",
  );
  return errors;
}
