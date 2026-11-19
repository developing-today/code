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

/** Thrown when the versions list is missing a version number, as it can for a few seconds after an upload. */
export class VersionListGap extends Error {}

/**
 * The first version between the live one and a code-only upload that
 * isn't itself a code-only upload, or null. A code-only upload inherits
 * ACCESS_TOKEN from the newest version before it, so any such version
 * means it may not carry the live token.
 *
 * Cloudflare numbers a Worker's versions 1, 2, 3, ... in upload order,
 * so the versions in between are exactly the numbers between the live
 * one and this upload. A number missing from the list means the list
 * hasn't caught up yet (it can lag an upload), and an unseen version
 * could be a full deploy, so that throws VersionListGap rather than
 * passing on a partial list.
 *
 * @param {{id: string, number: number, annotations?: Record<string, string>}[]} versions in any order
 * @param {string} liveId the live version
 * @param {string | null} uploadedId this run's version, once uploaded
 * @returns {{id: string} | null}
 * @throws {VersionListGap} when a version between the two is missing from the list
 * @throws {Error} when the live version (or this upload, if given) can't be placed
 */
export function strayBeforeCodeOnly(versions, liveId, uploadedId = null) {
  if (!versions.every((v) => Number.isSafeInteger(v.number))) {
    throw new Error("a version in the list has no version number; refusing to judge token inheritance");
  }
  const live = versions.find((v) => v.id === liveId);
  if (!live) throw new Error(`live version ${liveId} is not in the versions list`);
  let top = Math.max(...versions.map((v) => v.number)) + 1;
  if (uploadedId) {
    const mine = versions.find((v) => v.id === uploadedId);
    if (!mine || mine.number <= live.number) {
      throw new Error(`uploaded version ${uploadedId} is not newer than the live one`);
    }
    top = mine.number;
  }
  const byNumber = new Map(versions.map((v) => [v.number, v]));
  for (let n = top - 1; n > live.number; n--) {
    const v = byNumber.get(n);
    if (!v) throw new VersionListGap(`version number ${n} (between the live version and this upload) is not listed yet`);
    if (!(v.annotations?.["workers/message"] ?? "").includes(CODE_ONLY_MARK)) return v;
  }
  return null;
}

/**
 * The token fingerprint a code-only version must keep, from the live
 * Worker's /healthz answer. A code-only deploy has no token of its own,
 * so without the live fingerprint it can't check what it promotes and
 * refuses rather than trusting the versions list alone.
 *
 * @param {unknown} health parsed /healthz body, or null if unreadable
 * @returns {string}
 */
export function codeOnlyFingerprint(health) {
  if (!health?.ok) throw new Error("could not read the live /healthz before promoting");
  const fp = health.token_fingerprint;
  if (typeof fp !== "string" || !/^[0-9a-f]{16}$/.test(fp)) {
    throw new Error("the live version reports no token fingerprint, so a code-only deploy can't check the token it keeps");
  }
  return fp;
}

/**
 * Promotes a version and verifies it; if either step fails, puts the
 * previously live version back before rethrowing, so a failed or
 * unverifiable promotion never stays live. It only goes back while this
 * run's deployment is still the newest one, so it never overwrites a
 * deployment someone made after it (the Cloudflare API has no
 * conditional deploy, so a deployment landing in the instant between that
 * check and the rollback is the one case left to the operator).
 *
 * @param {{newId: string, previousId: string | null, promote: () => unknown,
 *   verify: () => Promise<unknown>, rollback: (id: string) => unknown}} steps
 */
export async function promoteOrRollBack({ newId, previousId, promote, verify, rollback, stillNewest = async () => true }) {
  if (!previousId) throw new Error("could not tell which version is live, so there would be nothing to roll back to; refusing to promote");
  try {
    await promote();
    await verify();
  } catch (err) {
    // A verify step can name a different version to go back to (another
    // deployment that went live just before this one).
    const target = err.rollbackTo ?? previousId;
    let ours = false;
    try {
      ours = await stillNewest();
    } catch {
      ours = false;
    }
    if (!ours) {
      // Someone else deployed after this run (or that can't be read): going
      // back now would overwrite their deployment, so leave it alone.
      err.message += `; not rolling back, because version ${newId} is no longer the newest deployment (or that couldn't be checked), so check the dashboard`;
    } else if (target !== newId) {
      try {
        await rollback(target);
        err.message += `; version ${target} was put back live`;
      } catch (rbErr) {
        err.message += `; putting version ${target} back live failed too (${rbErr.message}), so fix it by hand with --deploy-version ${target} --allow-secret-change`;
      }
    }
    throw err;
  }
}

/**
 * Checks that this promotion replaced the version the deploy read as live
 * just before promoting. If another deployment went live in between (say
 * a full deploy rotating the token), the token fingerprint read earlier no
 * longer describes what was replaced, so this returns the version that
 * really preceded this one (to go back to), or null when all is as read.
 *
 * @param {{versions?: {version_id: string, percentage: number}[]}[]} newestFirst deployments, newest first
 * @param {string} newId this run's version, which must be the newest deployment
 * @param {string} previousId the version read as live before promoting
 * @returns {string | null}
 * @throws {Error} when the newest deployment isn't this one or nothing preceded it
 */
export function replacedOtherThan(newestFirst, newId, previousId) {
  const only = (d) => (d?.versions?.length === 1 && d.versions[0].percentage === 100 ? d.versions[0].version_id : null);
  if (only(newestFirst[0]) !== newId) throw new Error(`the newest deployment is not version ${newId} alone`);
  const before = only(newestFirst[1]);
  if (!before) throw new Error(`could not tell which deployment version ${newId} replaced`);
  return before === previousId ? null : before;
}

/**
 * Whether a deployment is this run's: the given version alone at 100%,
 * and, once its deployment id has been seen, that same deployment. A
 * newer deployment that splits traffic, or serves the same version
 * again, is someone else's and must not be rolled back over.
 *
 * @param {{id?: string, versions?: {version_id: string, percentage: number}[]} | undefined} deployment
 * @param {string} versionId
 * @param {string | null} [deploymentId]
 * @returns {boolean}
 */
export function isOwnDeployment(deployment, versionId, deploymentId = null) {
  const vs = deployment?.versions;
  if (!Array.isArray(vs) || vs.length !== 1) return false;
  if (vs[0]?.version_id !== versionId || vs[0]?.percentage !== 100) return false;
  return deploymentId == null || deployment.id === deploymentId;
}

/**
 * What is wrong with the last /healthz answer after waiting for a
 * version to go live, or null when nothing is. A Worker built before
 * /healthz reported its version answers version null, which is accepted
 * unless the version is required; any other version is the wrong one.
 *
 * @param {{version?: string | null} | null} health
 * @param {string} id
 * @param {{requireVersion?: boolean}} [opts]
 * @returns {string | null}
 */
export function healthVersionProblem(health, id, { requireVersion = false } = {}) {
  if (health?.version === id) return null;
  if (health?.version != null) return `answered from version ${health.version}, not ${id}`;
  return requireVersion ? `never answered from version ${id}` : null;
}
