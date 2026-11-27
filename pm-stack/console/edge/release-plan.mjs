// The decisions release.mjs makes, apart from the doing: nothing here
// talks to Cloudflare or runs a command, so every one of them is tested
// (../test/release.test.ts).

const VERSION_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const SHA = /^[0-9a-f]{40}$/;
const WORKER_NAME = /^[a-z0-9][a-z0-9-]{0,62}$/;

/** What a commit is called once it is a version. */
export function names(sha) {
  if (!SHA.test(sha)) throw new Error("--sha must be a full commit id");
  const short = sha.slice(0, 12);
  return { short, full: sha, tag: `git-${short}`, message: `git ${sha}` };
}

/** The name and the newest migration tag a wrangler configuration gives.
 * The text is JSON with comments, and is read without a parser for
 * them: comments are dropped where they stand outside strings. */
export function configOf(text) {
  let out = "";
  let inString = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (inString) {
      out += c;
      if (c === "\\") out += text[++i] ?? "";
      else if (c === '"') inString = false;
    } else if (c === '"') {
      inString = true;
      out += c;
    } else if (c === "/" && text[i + 1] === "/") {
      while (i < text.length && text[i] !== "\n") i++;
      out += "\n";
    } else if (c === "/" && text[i + 1] === "*") {
      i = text.indexOf("*/", i + 2);
      if (i < 0) throw new Error("the configuration has a comment that never ends");
      i++;
    } else out += c;
  }
  const config = JSON.parse(out.replace(/,(\s*[}\]])/g, "$1"));
  if (typeof config.name !== "string" || !WORKER_NAME.test(config.name)) {
    throw new Error("the configuration names no Worker");
  }
  const migrations = Array.isArray(config.migrations) ? config.migrations : [];
  return { config, name: config.name, migrationTag: migrations.at(-1)?.tag ?? null };
}

/** The Worker a stand-up from nothing is tried on. It is never the real
 * one: it has a name of its own, made here and nowhere else. */
export function scratchName(base) {
  return `${base}-scratch`;
}

/** A Worker this script may delete: the scratch one, and no other. */
export function removable(name, base) {
  return name === scratchName(base) && name !== base;
}

/** What Cloudflare said about a Worker's existence, read strictly: only
 * a plain "there is no such Worker" counts as absent, because the answer
 * to absence is to deploy an empty Worker under that name. */
export function existence(status, body) {
  if (status === 200 && body?.success === true) {
    return { exists: true, migrationTag: body.result?.default_environment?.script?.migration_tag ?? null };
  }
  // The two codes Cloudflare gives for "no such Worker" (the ones
  // wrangler itself takes to mean that).
  const codes = Array.isArray(body?.errors) ? body.errors.map((e) => e?.code) : [];
  if (status === 404 && (codes.includes(10007) || codes.includes(10090))) {
    return { exists: false, migrationTag: null };
  }
  throw new Error(`cannot tell whether the Worker exists: http ${status} ${JSON.stringify(body?.errors ?? null)}`);
}

/** Whether a list of the account's Workers has one of this name. */
export function listed(body, name) {
  if (body?.success !== true || !Array.isArray(body.result)) {
    throw new Error("cannot read the account's list of Workers");
  }
  return body.result.some((script) => script?.id === name);
}

/** The steps to take, given what is there.
 *
 * - No Worker: stand an empty one up, then upload and promote.
 * - A Worker whose Durable Object classes are as the configuration has
 *   them: upload a version and promote it.
 * - A Worker whose classes differ (a new migration, or none applied
 *   yet): one full deploy, because a version cannot carry a migration.
 */
export function plan({ exists, migrationTag }, wantTag) {
  if (!exists) return ["stand-up", "upload", "promote"];
  if (migrationTag === wantTag) return ["upload", "promote"];
  return ["deploy"];
}

/** The id of the version wrangler just uploaded for this Worker, from
 * the file it writes its results to (one JSON object a line). */
export function uploadedVersion(text, name) {
  let found = null;
  for (const line of text.split("\n")) {
    if (line.trim() === "") continue;
    let entry;
    try {
      entry = JSON.parse(line);
    } catch {
      continue;
    }
    if (entry?.type === "version-upload" && entry.worker_name === name) found = entry.version_id;
  }
  if (typeof found !== "string" || !VERSION_ID.test(found)) {
    throw new Error("wrangler did not say which version it uploaded");
  }
  return found;
}

/** The version serving all traffic now, or null when there is none or
 * traffic is split (nothing to put back with one step). */
export function servingVersion(body) {
  const newest = body?.result?.deployments?.[0];
  const versions = Array.isArray(newest?.versions) ? newest.versions : [];
  if (versions.length !== 1 || versions[0]?.percentage !== 100) return null;
  const id = versions[0].version_id;
  return typeof id === "string" && VERSION_ID.test(id) ? id : null;
}

/** What a health answer means for a release of `full`.
 * The deployed version must report the exact commit it was deployed
 * from and a truthful age before it counts as this one. */
export function healthSays(answer, full, nowMs = Date.now()) {
  if (!answer || typeof answer !== "object") return "silent";
  if (answer.empty === true && answer.edge === true) return "empty";
  if (answer.ok !== true || answer.edge !== true) return "silent";
  if (answer.version !== full) return "another";
  if (typeof answer.deployedAt !== "string" || !Number.isInteger(answer.ageSeconds) || answer.ageSeconds < 0) {
    return "another";
  }
  const deployedAtMs = Date.parse(answer.deployedAt);
  if (!Number.isFinite(deployedAtMs) || deployedAtMs > nowMs) return "another";
  const expectedAgeSeconds = Math.floor((nowMs - deployedAtMs) / 1_000);
  return Math.abs(answer.ageSeconds - expectedAgeSeconds) <= 5 ? "this" : "another";
}

/** A version id as a person may give it for a rollback. */
export function isVersionId(value) {
  return typeof value === "string" && VERSION_ID.test(value);
}
