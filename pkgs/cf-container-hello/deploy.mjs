// Standalone deploy entrypoint for the known-good Cloudflare container probe.
// Image publication and Worker activation are separate commands by design:
//   deploy.mjs publish-image [--dry-run]
//   deploy.mjs activate [--dry-run]
// The probe uses Cloudflare's official hello-world image; there is no local
// image recipe to publish. `publish-image` uploads a Worker version with cf;
// `activate --version <id>` separately puts that existing version live.
//
// Every command-line form is checked before subprocesses or network access.
import { execFileSync } from "node:child_process";
import { parseArgs } from "./deploy-args.mjs";

let options;
try {
  options = parseArgs(process.argv.slice(2));
} catch (error) {
  console.error(`deploy.mjs: ${error.message}`);
  process.exit(2);
}

const WORKER = "cf-container-hello";
if (options.command === "publish-image") {
  const tag = requireEnv("CF_VERSION_TAG");
  const args = ["cf@1.0.0-beta.20", "workers", "versions", "create", "--worker-id", WORKER, "--tag", tag];
  if (options.dryRun) args.push("--dry-run");
  execFileSync("npx", args, { stdio: "inherit" });
  process.exit(0);
}
if (options.dryRun) {
  console.error("activation dry-run is a no-op; no Cloudflare API call made");
  process.exit(0);
}
if (!options.version) {
  console.error("activate requires --version <id> from publish-image output");
  process.exit(2);
}
const expectedTag = requireEnv("CF_VERSION_TAG");
const versionsOut = execFileSync("npx", ["cf@1.0.0-beta.20", "workers", "versions", "list", "--worker-id", WORKER], { encoding: "utf8" });
const listed = parseJsonOutput(versionsOut);
const available = Array.isArray(listed) ? listed : listed.result ?? listed.versions;
const target = available?.find((version) => version.id === options.version);
if (!target || target.annotations?.["workers/tag"] !== expectedTag) {
  throw new Error(`version ${options.version} is not an upload from this workflow commit (${expectedTag})`);
}
const versions = JSON.stringify([{ version_id: options.version, percentage: 100 }]);
execFileSync("npx", [
  "cf@1.0.0-beta.20", "workers", "deployments", "create",
  "--worker-id", WORKER, "--strategy", "percentage", "--versions", versions,
], { stdio: "inherit" });
const deploymentsOut = execFileSync("npx", ["cf@1.0.0-beta.20", "workers", "deployments", "list", "--worker-id", WORKER], { encoding: "utf8" });
const deploymentData = parseJsonOutput(deploymentsOut);
const deployments = Array.isArray(deploymentData) ? deploymentData : deploymentData.result ?? deploymentData.deployments;
const latest = deployments?.[0];
if (!latest?.versions?.some((version) => version.version_id === options.version && version.percentage === 100)) {
  throw new Error(`deployment readback did not confirm version ${options.version} at 100%`);
}

function requireEnv(name) {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required`);
  return value;
}

function parseJsonOutput(output) {
  try {
    return JSON.parse(output);
  } catch {
    throw new Error(`cf returned unexpected non-JSON output: ${output.slice(0, 200)}`);
  }
}
