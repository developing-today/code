// Does the message-bridge Worker exist yet? Kept apart from deploy.mjs
// so the rule is tested without running a deploy.
//
// `cf workers versions create` can't create a brand-new Worker, so the
// first deploy into an account bootstraps with `cf deploy`, which
// uploads the first version and puts it live at 100% in one step (the
// same limitation pkgs/pkarr-worker handles). That path skips the
// upload-then-promote checks, so it is chosen only on the API's own
// not-found answer for the script (HTTP 404 with error code 10007).
// Anything else (a bad or under-scoped token, an API blip, a network
// error) stops the deploy rather than quietly switching mechanism.

export const WORKER_NOT_FOUND_CODE = 10007;

/**
 * @param {number} status HTTP status of GET .../workers/scripts/<name>/settings
 * @param {unknown} body its parsed JSON body, or null if it wasn't JSON
 * @returns {boolean} true when the Worker exists, false only on not-found
 * @throws {Error} on any other answer
 */
export function classifyWorkerProbe(status, body) {
  const errors = Array.isArray(body?.errors) ? body.errors : [];
  if (status === 200 && body?.success === true) return true;
  if (status === 404 && errors.some((e) => e?.code === WORKER_NOT_FOUND_CODE)) return false;
  const detail = errors.map((e) => `${e?.code}: ${e?.message}`).join("; ") || "no error detail";
  throw new Error(
    `could not tell whether the Worker exists (HTTP ${status}, ${detail}); aborting before any upload`,
  );
}

export const CODE_ONLY_MARK = "via message-bridge-deploy workflow";

/**
 * The first version between the live one and a code-only upload that
 * isn't itself a code-only upload, or null. A code-only upload inherits
 * ACCESS_TOKEN from the newest version before it, so any such version
 * means it may not carry the live token.
 *
 * @param {{id: string, annotations?: Record<string, string>}[]} newestFirst versions, newest first
 * @param {string} liveId the live version
 * @param {string | null} uploadedId this run's version, once uploaded
 * @returns {{id: string} | null}
 * @throws {Error} when the live version (or this upload, if given) can't be placed
 */
export function strayBeforeCodeOnly(newestFirst, liveId, uploadedId = null) {
  const liveAt = newestFirst.findIndex((v) => v.id === liveId);
  if (liveAt === -1) throw new Error(`live version ${liveId} is not in the versions list`);
  let from = 0;
  if (uploadedId) {
    const mine = newestFirst.findIndex((v) => v.id === uploadedId);
    if (mine === -1 || mine > liveAt) throw new Error(`uploaded version ${uploadedId} is not newer than the live one`);
    from = mine + 1;
  }
  return (
    newestFirst.slice(from, liveAt).find((v) => !(v.annotations?.["workers/message"] ?? "").includes(CODE_ONLY_MARK)) ??
    null
  );
}
