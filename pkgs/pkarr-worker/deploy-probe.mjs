// Existence-probe classification for deploy.mjs, kept in its own module so
// the rules can be tested without importing the deploy script (which runs
// cf/bws at import time). The rule is fail-closed (#4865): the bootstrap
// `cf deploy` path deploys at 100% immediately, bypassing the versions
// flow, so it may be chosen ONLY when the Worker genuinely does not exist.
// Any other probe failure — an expired or under-scoped token, a
// Cloudflare API blip, a network error, a broken cf install — is a stop,
// never a silent change of deploy mechanism.

/**
 * The not-found shapes of `cf workers get <name>` for a Worker that does
 * not exist: the Cloudflare API error code 10007
 * (workers.api.error.not_found) or cf's rendering of it. Deliberately
 * narrow: a bare "not found" for something else (an account, a zone)
 * does not match, so it aborts like every other unrecognized failure.
 */
const NOT_FOUND_PATTERNS = [
  /10007/,
  /workers\.api\.error\.not_found/,
  /script not found/i,
  /could not find (the )?worker/i,
  /worker [^\n]* not found/i,
];

export function isNotFoundProbeFailure(output) {
  if (!output) return false;
  return NOT_FOUND_PATTERNS.some((pattern) => pattern.test(output));
}

/** Everything a failed execFileSync can tell us, in one string. */
export function probeFailureOutput(err) {
  return [err?.stderr, err?.stdout, err?.message]
    .filter((part) => typeof part === "string" && part.length > 0)
    .join("\n");
}

/**
 * Run the existence probe.
 *
 * @param {() => unknown} runGet performs `cf workers get <worker>` and
 *   throws on a non-zero exit or spawn failure
 * @param {string} worker worker name, for the abort message
 * @returns {boolean} true when the Worker exists; false only when the
 *   probe failed with the not-found response
 * @throws {Error} on any other probe failure, with the cf error visible
 */
export function probeWorkerExists(runGet, worker) {
  try {
    runGet();
    return true;
  } catch (err) {
    const output = probeFailureOutput(err);
    if (isNotFoundProbeFailure(output)) return false;
    throw new Error(
      `could not determine whether worker ${worker} exists; aborting before any upload. ` +
        `The bootstrap path is only for a genuinely new Worker. cf said:\n${output || String(err)}`,
    );
  }
}
