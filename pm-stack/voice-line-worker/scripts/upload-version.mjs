// Upload a prebuilt Worker Version without creating or changing a deployment.
import { execFileSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { parseVersionItems, versionMatchesProvenance } from "../src/deploy-contract.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const workerDir = resolve(here, "..");
const workerName = "voice-line";
const sha = process.env.GITHUB_SHA ?? execFileSync("git", ["rev-parse", "HEAD"], {
  cwd: workerDir,
  encoding: "utf8",
}).trim();
if (!/^[a-f0-9]{40}$/i.test(sha)) throw new Error("GITHUB_SHA must be a full 40-character commit SHA");
const runId = process.env.GITHUB_RUN_ID;
const runAttempt = process.env.GITHUB_RUN_ATTEMPT;
if (!/^\d+$/.test(runId) || !/^\d+$/.test(runAttempt)) {
  throw new Error("GITHUB_RUN_ID and GITHUB_RUN_ATTEMPT must identify the upload workflow run");
}
requiredEnv("CLOUDFLARE_ACCOUNT_ID");
requiredEnv("CLOUDFLARE_API_TOKEN");

const tag = `git-${sha}-r${runId}-a${runAttempt}`;
const message = `git ${sha}`;
runCf(["workers", "versions", "create", "--prebuilt", "--worker", workerName, "--tag", tag, "--message", message]);

const versions = parseVersionItems(runCf(["-q", "workers", "versions", "list", "--worker", workerName, "--per-page", "100"]));
const matches = versions.filter((version) => versionMatchesProvenance(version, sha, runId, runAttempt));
if (matches.length !== 1 || typeof matches[0]?.id !== "string") {
  throw new Error(`expected exactly one uploaded version with exact provenance for ${sha}; found ${matches.length}`);
}

console.log(`Uploaded candidate version: ${matches[0].id}`);
console.log(`Source commit: ${sha}`);
console.log(`Upload run: ${runId} (attempt ${runAttempt})`);
if (process.env.GITHUB_STEP_SUMMARY) {
  const fs = await import("node:fs/promises");
  await fs.appendFile(process.env.GITHUB_STEP_SUMMARY,
    `## Voice line Worker candidate uploaded\n\n- Version ID: \`${matches[0].id}\`\n- Source SHA: \`${sha}\`\n- Upload run: \`${runId}\` (attempt \`${runAttempt}\`)\n- Traffic changed: no\n`);
}

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
