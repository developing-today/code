// Argument parsing for deploy.mjs, kept apart so it is tested without
// running the deploy. Anything not understood fails closed (exit 2)
// before any credential is read, so a typo never turns into a live
// deploy.

export const USAGE =
  "usage: deploy.mjs [--dry-run | --print-urls | --code-only | --deploy-version <id> [--dry-run]]";

const VERSION_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

/**
 * @param {string[]} argv
 * @returns {{ mode: "deploy" | "print-urls" | "code-only" | "deploy-version", dryRun: boolean, deployVersion: string | null }}
 */
export function parseArgs(argv) {
  let dryRun = false;
  let printUrls = false;
  let codeOnly = false;
  let deployVersion = null;
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--dry-run" && !dryRun) dryRun = true;
    else if (a === "--print-urls" && !printUrls) printUrls = true;
    else if (a === "--code-only" && !codeOnly) codeOnly = true;
    else if (a === "--deploy-version" && deployVersion === null) {
      const id = argv[++i];
      if (!id || !VERSION_ID.test(id)) throw new Error("--deploy-version needs a version id (a UUID)");
      deployVersion = id;
    } else throw new Error(`unrecognized or repeated argument: ${a}`);
  }
  if (printUrls && argv.length > 1) throw new Error("--print-urls takes no other argument");
  if (codeOnly && argv.length > 1) throw new Error("--code-only takes no other argument");
  if (printUrls) return { mode: "print-urls", dryRun: false, deployVersion: null };
  if (codeOnly) return { mode: "code-only", dryRun: false, deployVersion: null };
  if (deployVersion) return { mode: "deploy-version", dryRun, deployVersion };
  return { mode: "deploy", dryRun, deployVersion: null };
}
