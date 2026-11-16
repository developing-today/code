// Argument parsing for deploy.mjs, kept in its own module so the rules can
// be tested without importing the deploy script (which runs cf/bws at import
// time). The rule is fail-closed: a deploy tool must never ignore a word it
// does not understand, because the operator may have asked for a preview or a
// rollback and would instead ship HEAD (#3264).

export const USAGE = [
  "usage: deploy.mjs [--dry-run]",
  "       deploy.mjs --deploy-version <id> [--dry-run]",
].join("\n");

/**
 * Parse deploy arguments.
 *
 * Accepted:
 *   --dry-run
 *   --deploy-version <id>
 *   --deploy-version=<id>
 *
 * Anything else throws, so the caller can abort before any bws/cf call.
 *
 * @param {string[]} argv arguments after the script name
 * @returns {{ dryRun: boolean, deployVersion: string | null }}
 */
export function parseArgs(argv) {
  let dryRun = false;
  let deployVersion = null;

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];

    if (arg === "--dry-run") {
      dryRun = true;
      continue;
    }

    if (arg === "--deploy-version") {
      const value = argv[i + 1];
      if (value === undefined || value === "" || value.startsWith("-")) {
        throw new Error(`--deploy-version needs a version id\n${USAGE}`);
      }
      deployVersion = value;
      i += 1;
      continue;
    }

    if (arg.startsWith("--deploy-version=")) {
      const value = arg.slice("--deploy-version=".length);
      if (value === "") {
        throw new Error(`--deploy-version needs a version id\n${USAGE}`);
      }
      deployVersion = value;
      continue;
    }

    throw new Error(`unrecognized argument ${JSON.stringify(arg)}\n${USAGE}`);
  }

  // --dry-run with --deploy-version is deliberately accepted: #3252 makes it
  // preview a rollback instead of creating the deployment.
  return { dryRun, deployVersion };
}
