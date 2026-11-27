// Validates config.json and uploads a Cloudflare Workers Version. Uploading does not promote it,
// change routes, or retire another Worker. Promotion and resource retirement are separate actions.
// Usage: upload-version.mjs [--dry-run]. Uploads require the dedicated mail-environment token.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const cfBin = join(here, "node_modules", ".bin", "cf");
const KNOWN_TRANSPORTS = ["email"];
const MODES = ["template", "free"];
const PLACEHOLDER = "{code}";

// Strict argument validation (#3281): the only accepted invocation is
// `upload-version.mjs [--dry-run]`. Anything else — a typo'd `--dry-rnu`, a
// stray positional, an unknown flag — aborts here, before any config,
// cf or API work: unrecognized arguments must never fall through to an upload.
const args = process.argv.slice(2);
const unknownArgs = args.filter((arg) => arg !== "--dry-run");
if (args.length > 1 || unknownArgs.length > 0) {
  // unknownArgs empty on this branch means every arg was --dry-run, so a
  // length failure is a duplicate, not an unrecognized argument (#7855).
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
const uploadToken = dryRun ? undefined : requireEnv("ID_MAIL_WORKER_CF_TOKEN_MAIL_WORKER_PRODUCTION");

// Install the locked toolchain in the separate credential-free workflow step.
// In an authenticated run never spawn a package installer.
if (!existsSync(cfBin)) {
  throw new Error("pinned cf CLI missing; install dependencies before supplying credentials");
}

const sha = process.env.GITHUB_SHA ?? execFileSync("git", ["rev-parse", "HEAD"], { cwd: here, encoding: "utf8" }).trim();
const runId = process.env.GITHUB_RUN_ID;
const runAttempt = process.env.GITHUB_RUN_ATTEMPT;
const provenancePath = dryRun ? undefined : requireEnv("ID_MAIL_WORKER_PROVENANCE_FILE");
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
const tag = dryRun ? `git-${sha.slice(0, 12)}` : `mail-${sha.slice(0, 12)}-run-${runId}-attempt-${runAttempt}`;
const cfArgs = [
  "workers", "versions", "create",
  "--worker", config.worker,
  "--tag", tag,
  "--message", `publish ${sha} for ${config.worker} (upload only)`,
];
if (dryRun) cfArgs.push("--dry-run");
let baselineDeployment;
if (!dryRun) {
  const baselineOutput = execFileSync(cfBin, ["workers", "deployments", "list", "--worker", config.worker, "--per-page", "1"], {
    cwd: here,
    encoding: "utf8",
    env: { ...process.env, CLOUDFLARE_API_TOKEN: uploadToken },
  });
  baselineDeployment = firstDeployment(unwrapJson(baselineOutput));
  if (typeof baselineDeployment?.versions?.[0]?.version_id !== "string" || !baselineDeployment.id) {
    throw new Error("cf could not identify the current active Worker deployment; refusing to upload");
  }
  const versionsOutput = execFileSync(cfBin, ["workers", "versions", "list", "--worker-id", config.worker, "--per-page", "1"], {
    cwd: here,
    encoding: "utf8",
    env: { ...process.env, CLOUDFLARE_API_TOKEN: uploadToken },
  });
  const latestVersion = firstVersion(unwrapJson(versionsOutput));
  if (latestVersion?.id !== baselineDeployment.versions[0].version_id) {
    throw new Error("latest uploaded version is not the current active version; refusing to inherit CALLERS from an unpromoted version");
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
  const version = unwrapJson(output);
  const versionId = version.id ?? version.version_id;
  if (typeof versionId !== "string" || !versionId) {
    throw new Error("cf did not return the uploaded version ID; refusing to emit provenance");
  }
  if (version.annotations?.["workers/tag"] && version.annotations["workers/tag"] !== tag) {
    throw new Error("cf returned an uploaded version with an unexpected tag");
  }
  const deploymentOutput = execFileSync(cfBin, ["workers", "deployments", "list", "--worker", config.worker, "--per-page", "1"], {
    cwd: here,
    encoding: "utf8",
    env: { ...process.env, CLOUDFLARE_API_TOKEN: uploadToken },
  });
  const deployment = firstDeployment(unwrapJson(deploymentOutput));
  if (deployment?.id !== baselineDeployment.id || deployment?.versions?.[0]?.version_id !== baselineDeployment.versions[0].version_id) {
    throw new Error("the active deployment changed during upload; refusing to emit provenance");
  }
  const provenance = {
    schema: 1,
    repository: process.env.GITHUB_REPOSITORY,
    workflow: ".github/workflows/id-mail-worker-publish.yml",
    event: process.env.GITHUB_EVENT_NAME,
    ref: process.env.GITHUB_REF,
    sha,
    runId,
    runAttempt,
    worker: config.worker,
    tag,
    versionId,
    baselineDeploymentId: baselineDeployment.id,
    baselineVersionId: baselineDeployment.versions[0].version_id,
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
    if (!ok) {
      errors.push(message);
    }
  };
  const isEmail = (value) => typeof value === "string" && /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(value);
  const isStringList = (value) => Array.isArray(value) && value.every((item) => typeof item === "string");
  const isPositiveInt = (value) => Number.isInteger(value) && value > 0;

  check(/^[a-z0-9-]+$/.test(c.worker ?? ""), "worker must be a lowercase name");
  check(/^\d{4}-\d{2}-\d{2}$/.test(c.compatibilityDate ?? ""), "compatibilityDate must be YYYY-MM-DD");
  check(typeof c.workersDev === "boolean", "workersDev must be true or false");
  check(isStringList(c.routes) && c.routes.length > 0, "routes must be a non-empty list of hostnames");
  check(isStringList(c.retire ?? []) && !(c.retire ?? []).includes(c.worker), "retire must list other Workers");
  check(isStringList(c.senders) && c.senders.length > 0 && c.senders.every(isEmail), "senders must be email addresses");
  check(isStringList(c.transports) && c.transports.length > 0, "transports must be a non-empty list");
  const transportList = Array.isArray(c.transports) ? c.transports : [];
  check(
    transportList.every((name) => KNOWN_TRANSPORTS.includes(name)),
    `transports must be among ${KNOWN_TRANSPORTS.join(", ")}`,
  );
  check(transportList.includes(c.defaultTransport), "defaultTransport must be one of transports");
  check(isPositiveInt(c.limits?.windowSeconds), "limits.windowSeconds must be a positive integer");
  check(isPositiveInt(c.limits?.callerPerWindow), "limits.callerPerWindow must be a positive integer");
  check(isPositiveInt(c.limits?.recipientPerWindow), "limits.recipientPerWindow must be a positive integer");
  check(Array.isArray(c.templates), "templates must be a list");
  for (const [index, template] of (Array.isArray(c.templates) ? c.templates : []).entries()) {
    if (template === null || typeof template !== "object") {
      check(false, `templates[${index}] must be an object with subject and body`);
      continue;
    }
    check(
      typeof template.subject === "string" && !/[\r\n]/.test(template.subject),
      "each template subject must be one line of text",
    );
    check(
      typeof template.body === "string" && template.body.includes(PLACEHOLDER),
      `each template body must contain ${PLACEHOLDER}`,
    );
  }
  check(c.callers && typeof c.callers === "object" && Object.keys(c.callers).length > 0, "callers must not be empty");
  for (const [name, caller] of Object.entries(c.callers ?? {})) {
    if (caller === null || typeof caller !== "object") {
      check(false, `callers.${name} must be an object`);
      continue;
    }
    check(typeof caller.tokenKey === "string", `callers.${name}.tokenKey must be a Bitwarden secret name`);
    const grants = Object.entries(caller.transports ?? {});
    check(grants.length > 0, `callers.${name}.transports must grant at least one transport`);
    for (const [transport, grant] of grants) {
      const where = `callers.${name}.transports.${transport}`;
      if (grant === null || typeof grant !== "object") {
        check(false, `${where} must be an object`);
        continue;
      }
      check(transportList.includes(transport), `${where} is not an enabled transport`);
      check(MODES.includes(grant.mode), `${where}.mode must be ${MODES.join(" or ")}`);
      check(
        isStringList(grant.from) && grant.from.length > 0 && grant.from.every((from) => c.senders?.includes(from)),
        `${where}.from must be non-empty and within senders`,
      );
      check(isPositiveInt(grant.maxRecipients), `${where}.maxRecipients must be a positive integer`);
      check(grant.mode !== "template" || (c.templates ?? []).length > 0, `${where} needs templates in template mode`);
    }
  }
  return errors;
}
