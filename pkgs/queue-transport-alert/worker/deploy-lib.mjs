// Pure helpers for deploy.mjs (code#6369), kept apart so they are tested
// without running cf or bws. Run directly (`node deploy-lib.mjs ARGS`) it
// only checks the arguments, which deploy.sh does before Bitwarden or
// sops are touched.
import { fileURLToPath } from "node:url";
import { join } from "node:path";

export const USAGE = [
  "usage: deploy.mjs [--dry-run]",
  "       deploy.mjs --deploy-version <id> [--dry-run]",
].join("\n");

// Fail closed: a word we do not understand must never turn into a live
// deploy of HEAD when a preview or a rollback was meant.
export function parseArgs(argv) {
  let dryRun = false;
  let deployVersion = null;
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--dry-run") {
      if (dryRun) throw new Error(`duplicate argument --dry-run\n${USAGE}`);
      dryRun = true;
    } else if (arg === "--deploy-version" || arg.startsWith("--deploy-version=")) {
      if (deployVersion !== null) throw new Error(`duplicate argument --deploy-version\n${USAGE}`);
      const value = arg === "--deploy-version" ? argv[(i += 1)] : arg.slice("--deploy-version=".length);
      if (!value || value.startsWith("-")) throw new Error(`--deploy-version needs a version id\n${USAGE}`);
      deployVersion = value;
    } else {
      throw new Error(`unrecognized argument ${JSON.stringify(arg)}\n${USAGE}`);
    }
  }
  return { dryRun, deployVersion };
}

// cf prints JSON by default, but progress lines can come first; take
// the first parseable JSON value that runs to the end of the output.
export function parseJsonOutput(out) {
  const text = String(out).trim();
  try {
    return JSON.parse(text);
  } catch { /* scan below */ }
  for (const open of ["{", "["]) {
    for (let i = text.indexOf(open); i >= 0; i = text.indexOf(open, i + 1)) {
      try {
        return JSON.parse(text.slice(i));
      } catch { /* keep scanning */ }
    }
  }
  throw new Error(`could not parse cf JSON output: ${text.slice(0, 200)}`);
}

export function listItems(parsed, what) {
  for (const candidate of [parsed, parsed?.result, parsed?.result?.items, parsed?.items, parsed?.[what]]) {
    if (Array.isArray(candidate)) return candidate;
  }
  throw new Error(`unexpected ${what} list output`);
}

function createdOn(item) {
  const raw = item?.metadata?.created_on ?? item?.created_on ?? item?.createdOn ?? 0;
  const parsed = typeof raw === "number" ? raw : Date.parse(raw);
  return Number.isFinite(parsed) ? parsed : 0;
}

// Uploads are not deduplicated, so a redeploy of the same commit adds a
// second version under the same tag: take the newest match.
export function newestWithTag(versions, tag) {
  return versions
    .filter((v) => (v.annotations?.["workers/tag"] ?? v.metadata?.annotations?.["workers/tag"]) === tag)
    .sort((a, b) => createdOn(b) - createdOn(a))[0] ?? null;
}

// The latest deployment must serve exactly this version at 100%.
export function servingCheck(deployments, versionId) {
  const latest = [...deployments].sort((a, b) => createdOn(b) - createdOn(a))[0];
  const active = (latest?.versions ?? []).filter((v) => Number(v.percentage) > 0);
  if (active.length === 1 && active[0].version_id === versionId && Number(active[0].percentage) === 100) {
    return { ok: true };
  }
  const serving = active.length ? active.map((v) => `${v.version_id}@${v.percentage}%`).join(", ") : "no active version";
  return { ok: false, serving };
}

// Return one unambiguous 100% serving version for code-only upload inheritance.
export function singleServingVersion(deployments) {
  const latest = [...deployments].sort((a, b) => createdOn(b) - createdOn(a))[0];
  const active = (latest?.versions ?? []).filter((v) => Number(v.percentage) > 0);
  return active.length === 1 && Number(active[0].percentage) === 100 && active[0].version_id
    ? active[0].version_id
    : null;
}

const VERSION_UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

export function ghaUploadIdentity({ sha, runId, attempt }) {
  if (!/^[0-9a-f]{40}$/.test(sha ?? "") || !/^[0-9]{1,20}$/.test(runId ?? "") ||
      !/^[0-9]{1,10}$/.test(attempt ?? "")) {
    throw new Error("invalid GitHub upload provenance");
  }
  return { tag: sha, message: "queue-transport-alert upload " + sha + " run " + runId + " attempt " + attempt };
}

// Pinned cf 1.0.0-beta.20 emits this machine record to WRANGLER_OUTPUT_FILE_PATH.
export function uploadedVersionId(raw, worker) {
  const item = JSON.parse(raw);
  if (item?.type !== "version-upload" || item.version !== 1 || item.worker_name !== worker ||
      item.worker_name_overridden !== false || item.wrangler_environment ||
      !VERSION_UUID.test(item.version_id ?? "")) {
    throw new Error("cf upload did not return a version UUID for the exact Worker");
  }
  return item.version_id;
}

export function hasExactUpload(versions, id, { tag, message }) {
  if (!VERSION_UUID.test(id ?? "")) return false;
  const matches = versions.filter((item) => item.id === id);
  if (matches.length !== 1) return false;
  const annotations = matches[0].annotations ?? matches[0].metadata?.annotations;
  return annotations?.["workers/tag"] === tag && annotations?.["workers/message"] === message;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    parseArgs(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    process.exit(2);
  }
}

// The home-resolution contract (#5762/#8220 unit 3, #14757), shaped after
// bin/home-dir.mjs: home is a non-empty HOME; unset or empty is a named
// error at the first surface that needs a path — no passwd fallback
// (os.homedir() falls back to the passwd entry when HOME is unset, which
// is exactly the substitute directory the contract forbids).
export class HomeUnsetError extends Error {
  constructor(purpose) {
    super(
      `HOME is not set or is empty: ${purpose} requires a non-empty HOME ` +
        "(home-resolution contract, issues #5762/#8220)",
    );
    this.name = "HomeUnsetError";
  }
}

/** The non-empty HOME, or a HomeUnsetError naming `purpose`. */
export function requireHome(env = process.env, purpose = "this tool") {
  const home = env.HOME;
  if (typeof home !== "string" || home === "") throw new HomeUnsetError(purpose);
  return home;
}

// The broker token file the deploy reads: QTA_BROKER_TOKEN_FILE, else the
// default under HOME. The token FILE is optional (absent = the previous
// version's BROKER_AGENT_TOKEN carries over), but resolving its default
// path is not: a deploy whose HOME is unset or empty fails here with the
// named error rather than silently treating the token as absent and
// pinning the previous secret by accident (#14757 decision, recorded in
// the PR that landed this).
export function resolveBrokerTokenFile(env = process.env) {
  return (
    env.QTA_BROKER_TOKEN_FILE ??
    join(requireHome(env, "the queue broker token file (QTA_BROKER_TOKEN_FILE)"), ".config", "queue-broker", "token")
  );
}
