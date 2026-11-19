// Argument parsing for deploy.mjs, kept apart so it is tested without
// running the deploy. Anything not understood fails closed (exit 2)
// before any credential is read, so a typo never turns into a live
// deploy.

export const USAGE =
  "usage: deploy.mjs [--dry-run | --print-urls | --code-only | --deploy-version <id> [--allow-secret-change] [--dry-run]]";

const VERSION_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

/**
 * @param {string[]} argv
 * --allow-secret-change goes only with --deploy-version: Cloudflare blocks
 * deploying an older version when a secret has changed since it was
 * uploaded, and this flag is the explicit, per-run choice to deploy it
 * anyway (the Worker then serves that version's ACCESS_TOKEN).
 *
 * @returns {{ mode: "deploy" | "print-urls" | "code-only" | "deploy-version", dryRun: boolean, deployVersion: string | null, allowSecretChange: boolean }}
 */
export function parseArgs(argv) {
  let dryRun = false;
  let printUrls = false;
  let codeOnly = false;
  let deployVersion = null;
  let allowSecretChange = false;
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--dry-run" && !dryRun) dryRun = true;
    else if (a === "--print-urls" && !printUrls) printUrls = true;
    else if (a === "--code-only" && !codeOnly) codeOnly = true;
    else if (a === "--allow-secret-change" && !allowSecretChange) allowSecretChange = true;
    else if (a === "--deploy-version" && deployVersion === null) {
      const id = argv[++i];
      if (!id || !VERSION_ID.test(id)) throw new Error("--deploy-version needs a version id (a UUID)");
      deployVersion = id;
    } else throw new Error(`unrecognized or repeated argument: ${a}`);
  }
  if (printUrls && argv.length > 1) throw new Error("--print-urls takes no other argument");
  if (codeOnly && argv.length > 1) throw new Error("--code-only takes no other argument");
  if (allowSecretChange && !deployVersion) throw new Error("--allow-secret-change goes only with --deploy-version");
  const none = { dryRun: false, deployVersion: null, allowSecretChange: false };
  if (printUrls) return { ...none, mode: "print-urls" };
  if (codeOnly) return { ...none, mode: "code-only" };
  if (deployVersion) return { mode: "deploy-version", dryRun, deployVersion, allowSecretChange };
  return { ...none, mode: "deploy", dryRun };
}
