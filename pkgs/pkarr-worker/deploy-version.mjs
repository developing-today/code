// The --deploy-version branch of deploy.mjs, kept in its own module so
// the flag x path matrix can be tested without importing the deploy
// script (which runs cf/bws at import time). The rule is contract
// rule 4 of #3352: a parsed safety flag is honored on every path. The
// HEAD paths hand --dry-run to cf, but this branch creates the
// deployment itself, so dry-run must stop it BEFORE createDeployment:
// a rollback preview must never create a real 100% deployment (#3252).

/**
 * Deploy (or preview deploying) an existing version at 100%.
 *
 * @param {object} input
 * @param {boolean} input.dryRun preview only: report and create nothing
 * @param {string} input.deployVersion existing version id to deploy
 * @param {string} input.worker worker name, for the report lines
 * @param {string} input.workerUrl worker URL, for the live line
 * @param {(versionId: string) => void} input.createDeployment creates
 *   the 100% deployment (the mutation dry-run must never reach)
 * @param {(versionId: string) => Promise<void>} input.reportLive
 *   verifies and reports the live deployment
 * @param {(message: string) => void} [input.log] report sink
 * @returns {Promise<{ deployed: boolean, dryRun: boolean, versionId: string }>}
 */
export async function deployExistingVersion({
  dryRun,
  deployVersion,
  worker,
  workerUrl,
  createDeployment,
  reportLive,
  log = console.log,
}) {
  if (dryRun) {
    log(
      `dry run: would create a deployment of worker ${worker} at version ${deployVersion} (100%); no deployment created`,
    );
    return { deployed: false, dryRun: true, versionId: deployVersion };
  }
  createDeployment(deployVersion);
  await reportLive(deployVersion);
  log(`live: ${workerUrl} at version ${deployVersion}`);
  return { deployed: true, dryRun: false, versionId: deployVersion };
}
