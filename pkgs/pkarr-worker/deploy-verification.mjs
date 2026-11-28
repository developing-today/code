// Post-deploy checks are isolated from CLI/process setup so they can be
// tested with deployment and HTTP responses supplied by the caller.
/**
 * @param {{
 *   versionId: string,
 *   deployments: Array<{ created_on?: string | number, createdOn?: string | number, metadata?: { created_on?: string | number }, versions?: Array<{ version_id?: string, percentage?: number | string }> }>,
 *   healthUrl: string,
 *   fetchHealth: (url: string) => Promise<{ ok: boolean, status?: number, text: () => Promise<string> }>
 * }} input
 * @returns {Promise<{ versionId: string, healthUrl: string }>}
 */
export async function verifyDeployment({ versionId, deployments, healthUrl, fetchHealth }) {
  const latest = latestDeployment(deployments);
  const active = (latest?.versions ?? []).filter((version) => Number(version.percentage) > 0);
  const expectedIsServing =
    active.length === 1 &&
    active[0]?.version_id === versionId &&
    Number(active[0]?.percentage) === 100;

  if (!expectedIsServing) {
    const serving = active.length
      ? active.map((version) => `${version.version_id}@${version.percentage}%`).join(", ")
      : "no active version";
    throw new Error(
      `post-deploy verification failed: expected version ${versionId} at 100%, latest deployment serves ${serving}`
    );
  }

  let response;
  try {
    response = await fetchHealth(healthUrl);
  } catch (error) {
    throw new Error(`post-deploy health check request failed for ${healthUrl}: ${error.message}`);
  }
  if (!response?.ok) {
    throw new Error(
      `post-deploy health check failed for ${healthUrl}: HTTP ${response?.status ?? "no response"}`
    );
  }

  let body;
  try {
    body = (await response.text()).trim();
  } catch (error) {
    throw new Error(`post-deploy health check could not read ${healthUrl}: ${error.message}`);
  }
  if (body !== "ok") {
    throw new Error(`post-deploy health check returned an unexpected body from ${healthUrl}`);
  }

  return { versionId, healthUrl };
}

/**
 * @param {Array<{ created_on?: string | number, createdOn?: string | number, metadata?: { created_on?: string | number }, versions?: Array<{ version_id?: string, percentage?: number | string }> }>} deployments
 */
function latestDeployment(deployments) {
  if (!Array.isArray(deployments) || deployments.length === 0) return null;
  return [...deployments].sort((a, b) => deploymentTime(b) - deploymentTime(a))[0];
}

/** @param {{ created_on?: string | number, createdOn?: string | number, metadata?: { created_on?: string | number } }} deployment */
function deploymentTime(deployment) {
  const raw = deployment?.metadata?.created_on ?? deployment?.created_on ?? deployment?.createdOn ?? 0;
  const parsed = typeof raw === "number" ? raw : Date.parse(raw);
  return Number.isFinite(parsed) ? parsed : 0;
}
