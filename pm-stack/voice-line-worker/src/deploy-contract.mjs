/** Parse cf's JSON output even when it prefixes human-readable progress. */
export function parseCfJson(output) {
  const text = output.trim();
  try {
    return JSON.parse(text);
  } catch {
    for (const marker of ["{", "["]) {
      for (let index = text.indexOf(marker); index >= 0; index = text.indexOf(marker, index + 1)) {
        try {
          return JSON.parse(text.slice(index));
        } catch {
          // Continue until the first complete JSON value is found.
        }
      }
    }
    throw new Error(`could not parse cf JSON output: ${text.slice(0, 200)}`);
  }
}

function itemsFrom(parsed, label, output) {
  if (Array.isArray(parsed)) return parsed;
  if (Array.isArray(parsed?.result)) return parsed.result;
  if (Array.isArray(parsed?.result?.items)) return parsed.result.items;
  if (Array.isArray(parsed?.items)) return parsed.items;
  if (Array.isArray(parsed?.[label])) return parsed[label];
  throw new Error(`unexpected ${label} output: ${output.slice(0, 200)}`);
}

export function parseVersionItems(output) {
  return itemsFrom(parseCfJson(output), "versions", output);
}

export function parseDeploymentItems(output) {
  return itemsFrom(parseCfJson(output), "deployments", output);
}

function versionTag(version) {
  return version.annotations?.["workers/tag"]
    ?? version.metadata?.annotations?.["workers/tag"]
    ?? version.metadata?.tag;
}

function createdAt(version) {
  const raw = version.metadata?.created_on ?? version.created_on ?? version.createdOn ?? 0;
  const time = typeof raw === "number" ? raw : Date.parse(raw);
  return Number.isFinite(time) ? time : 0;
}

export function versionForTag(versions, tag) {
  return versions.filter((version) => version.id && versionTag(version) === tag)
    .sort((left, right) => createdAt(right) - createdAt(left))[0] ?? null;
}

export function deployableVersionIds(payload) {
  if (payload?.success === false) throw new Error("Cloudflare rejected the deployable-version check");
  const result = payload?.result;
  const candidates = Array.isArray(result) ? result : (result?.items ?? []);
  if (!Array.isArray(candidates)) throw new Error("unexpected deployable-version response shape");
  return candidates.flatMap((version) => typeof version?.id === "string" ? [version.id] : []);
}

export function deploymentIsFullRollout(deployments, versionId) {
  const latest = deployments[0];
  if (!Array.isArray(latest?.versions) || latest.versions.length !== 1) return false;
  const [version] = latest.versions;
  return version?.version_id === versionId && Number(version.percentage) === 100;
}

export function parseDeployArgs(argv) {
  if (argv.length > 0) throw new Error(`unexpected argument: ${argv[0]}`);
  return {};
}
