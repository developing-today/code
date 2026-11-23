// Promote exactly one previously uploaded desktop-commander Worker
// version to 100% of traffic. The version must come from a provenance
// manifest emitted by upload-version.mjs in a successful publish run
// that verify-upload-run.mjs has already vouched for, and the live
// deployment must still sit at the manifest's baseline — any drift
// aborts the promotion instead of guessing.
// Usage: promote.mjs --manifest <provenance.json> --promote
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const cfBin = join(here, "node_modules", ".bin", "cf");

const args = process.argv.slice(2);
const manifestFlag = args.indexOf("--manifest");
const manifestPath = manifestFlag >= 0 ? args[manifestFlag + 1] : undefined;
const rest = args.filter((_, i) => i !== manifestFlag && i !== manifestFlag + 1);
if (!manifestPath || rest.length !== 1 || rest[0] !== "--promote") {
  console.error("usage: promote.mjs --manifest <provenance.json> --promote");
  process.exit(2);
}

const config = JSON.parse(readFileSync(join(here, "config.json"), "utf8"));
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
const verified = JSON.parse(readFileSync(requireEnv("DC_WORKER_VERIFIED_RUN_FILE"), "utf8"));
const token = requireEnv("DC_WORKER_CF_TOKEN");
requireEnv("CLOUDFLARE_ACCOUNT_ID");

if (!existsSync(cfBin)) {
  throw new Error("pinned cf CLI missing; install dependencies before supplying credentials");
}
if (manifest.worker !== config.worker) {
  throw new Error(`manifest worker ${manifest.worker} does not match config worker ${config.worker}`);
}
if (manifest.sha !== verified.sha || String(manifest.runId) !== String(verified.id) || String(manifest.runAttempt) !== String(verified.attempt)) {
  throw new Error("manifest does not match the verified upload run; refusing to promote");
}
if (manifest.repository?.toLowerCase() !== process.env.GITHUB_REPOSITORY?.toLowerCase()) {
  throw new Error("manifest repository does not match this repository; refusing to promote");
}
if (typeof manifest.versionId !== "string" || !manifest.versionId) {
  throw new Error("manifest has no versionId; refusing to promote");
}

function cfJson(argv) {
  const output = execFileSync(cfBin, argv, {
    cwd: here,
    encoding: "utf8",
    env: { ...process.env, CLOUDFLARE_API_TOKEN: token },
  });
  let parsed;
  try {
    parsed = JSON.parse(output);
  } catch {
    throw new Error("cf returned non-JSON output; refusing to continue");
  }
  return parsed?.result ?? parsed;
}

const version = cfJson(["workers", "versions", "get", manifest.versionId, "--worker", config.worker]);
if ((version.id ?? version.version_id) !== manifest.versionId) {
  throw new Error("cf did not return the manifest version; refusing to promote");
}
if (version.annotations?.["workers/tag"] && version.annotations["workers/tag"] !== manifest.tag) {
  throw new Error("the version's tag does not match the manifest tag; refusing to promote");
}

const deployments = cfJson(["workers", "deployments", "list", "--worker", config.worker, "--per-page", "1"]);
const current = Array.isArray(deployments) ? deployments[0] : deployments?.deployments?.[0];
const currentVersionId = current?.versions?.[0]?.version_id ?? null;
if (currentVersionId !== manifest.baselineVersionId) {
  throw new Error(
    `active version ${currentVersionId} is not the manifest baseline ${manifest.baselineVersionId}; refusing to promote over drift`,
  );
}

const created = cfJson([
  "workers", "deployments", "create",
  "--worker", config.worker,
  "--strategy", "percentage",
  "--versions", JSON.stringify([{ version_id: manifest.versionId, percentage: 100 }]),
]);
const deploymentId = created?.id ?? created?.deployment_id;
console.log(`promoted ${config.worker} version ${manifest.versionId} (tag ${manifest.tag}) to 100%${deploymentId ? `; deployment ${deploymentId}` : ""}`);

function requireEnv(name) {
  const value = process.env[name];
  if (!value) {
    throw new Error(`${name} is not set`);
  }
  return value;
}
