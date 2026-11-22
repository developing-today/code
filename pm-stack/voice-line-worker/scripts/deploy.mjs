// Upload and deploy a prebuilt Worker Version through Cloudflare's cf CLI.
// The Build Output is created separately so the exact artifact that passes
// the offline build check is the one uploaded here.
import { execFileSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  deployableVersionIds,
  deploymentIsFullRollout,
  parseCfJson,
  parseDeployArgs,
  parseDeploymentItems,
  parseVersionItems,
  versionForTag,
} from "../src/deploy-contract.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const workerDir = resolve(here, "..");
const workerName = "voice-line";
const workerUrl = "https://voice-line.developing-today.workers.dev";

try {
  parseDeployArgs(process.argv.slice(2));
} catch (error) {
  console.error(`deploy.mjs: ${error.message}`);
  process.exit(2);
}

const accountId = requiredEnv("CLOUDFLARE_ACCOUNT_ID");
const apiToken = requiredEnv("CLOUDFLARE_API_TOKEN");
const sha = process.env.GITHUB_SHA ?? execFileSync("git", ["rev-parse", "HEAD"], {
  cwd: workerDir,
  encoding: "utf8",
}).trim();
if (!/^[a-f0-9]{40}$/i.test(sha)) throw new Error("GITHUB_SHA must be a full 40-character commit SHA");

const tag = `git-${sha.slice(0, 12)}`;
const message = `deploy ${sha} via GitHub Actions`;

runCf(["workers", "versions", "create", "--prebuilt", "--worker", workerName, "--tag", tag, "--message", message]);

const versionOutput = runCf(["-q", "workers", "versions", "list", "--worker", workerName, "--per-page", "100"]);
const version = versionForTag(parseVersionItems(versionOutput), tag);
if (!version) throw new Error(`uploaded version with tag ${tag} was not found`);
console.log(`uploaded version ${version.id}`);

const response = await fetch(
  `https://api.cloudflare.com/client/v4/accounts/${encodeURIComponent(accountId)}/workers/scripts/${encodeURIComponent(workerName)}/versions?deployable=true`,
  { headers: { authorization: `Bearer ${apiToken}` }, signal: AbortSignal.timeout(15_000) },
);
if (!response.ok) throw new Error(`deployable-version check failed with HTTP ${response.status}`);
const deployablePayload = await response.json();
if (!deployableVersionIds(deployablePayload).includes(version.id)) {
  throw new Error(`version ${version.id} is not in the deployable set; refusing to create a deployment`);
}

runCf([
  "-q", "workers", "deployments", "create",
  "--worker", workerName,
  "--strategy", "percentage",
  "--versions", JSON.stringify([{ version_id: version.id, percentage: 100 }]),
]);

const deploymentsOutput = runCf(["-q", "workers", "deployments", "list", "--worker", workerName]);
const deployments = parseDeploymentItems(deploymentsOutput);
if (!deploymentIsFullRollout(deployments, version.id)) {
  throw new Error(`deployment read-back did not show version ${version.id} serving 100% of traffic`);
}

// A GET to /health checks only the HTTP path and Turso read path; it never
// opens a WebSocket or invokes speech models.
const healthResponse = await fetch(`${workerUrl}/health`, { signal: AbortSignal.timeout(15_000) });
if (!healthResponse.ok) throw new Error(`/health returned HTTP ${healthResponse.status}`);
let health;
try {
  health = await healthResponse.json();
} catch {
  throw new Error("/health did not return JSON");
}
if (health?.ok !== true) throw new Error("/health response did not report ok: true");

console.log(`verified: version ${version.id} serves 100% of traffic and /health reports ok`);

function runCf(args) {
  const binary = resolve(workerDir, "node_modules/cf/bin/cf");
  return execFileSync(process.execPath, [binary, ...args], {
    cwd: workerDir,
    env: process.env,
    encoding: "utf8",
    stdio: ["inherit", "pipe", "inherit"],
  });
}

function requiredEnv(name) {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required`);
  return value;
}
