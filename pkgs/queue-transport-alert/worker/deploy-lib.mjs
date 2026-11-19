// Pure helpers for deploy.mjs (code#6369), kept apart so they are tested
// without running cf or bws. Run directly (`node deploy-lib.mjs ARGS`) it
// only checks the arguments, which deploy.sh does before Bitwarden or
// sops are touched.
import { fileURLToPath } from "node:url";

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

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    parseArgs(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    process.exit(2);
  }
}
