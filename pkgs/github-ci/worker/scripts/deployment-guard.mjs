#!/usr/bin/env node
import { readFileSync, appendFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";

const SECRET_BINDINGS = [
  "GITHUB_WEBHOOK_SECRET",
  "TURSO_PLATFORM_TOKEN",
  "TURSO_QUEUE_TOKEN",
  "EVENTS_BROKER_TOKEN",
  "EVENTS_BROKER_ACCESS_CLIENT_ID",
  "EVENTS_BROKER_ACCESS_CLIENT_SECRET",
];
const EXPECTED_TEXT_BINDINGS = {
  TURSO_PLATFORM_URL: "https://fleet-platform-dezren39.aws-us-east-1.turso.io",
  TURSO_QUEUE_URL: "https://fleet-queue-dezren39.aws-us-east-1.turso.io",
  EVENTS_BROKER_URL: "https://queue-broker.security.cab",
};

const VERSION_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const COMMIT_SHA = /^[0-9a-f]{40}$/i;

function usage() {
  throw new Error(
    "usage: deployment-guard.mjs build <build-output-v1-dir> | select-upload <versions.json> <commit-sha> | verify-version <version.json> <version-id> [commit-sha] | verify-deployable <deployable.json> <version-id> | verify-deployment <deployments.json> <version-id>",
  );
}

function readJson(path) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch {
    throw new Error(`could not read valid JSON from ${path}`);
  }
}

function unwrap(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return value;
  if (value.success === false) throw new Error("Cloudflare API response was unsuccessful");
  return Object.hasOwn(value, "result") ? value.result : value;
}

function rows(value, label) {
  let result = unwrap(value);
  if (Array.isArray(result)) return result;
  if (result && typeof result === "object") {
    if (Array.isArray(result.items)) return result.items;
    if (Array.isArray(result.versions)) return result.versions;
    if (Array.isArray(result.deployments)) return result.deployments;
  }
  throw new Error(`${label} response did not contain a list`);
}

function newest(items, label) {
  if (!items.length) throw new Error(`${label} response did not contain any records`);
  return items
    .map((item) => {
      const createdOn = item.created_on ?? item.metadata?.created_on;
      const createdAt = typeof createdOn === "string" ? Date.parse(createdOn) : Number.NaN;
      if (!Number.isFinite(createdAt)) throw new Error(`${label} record is missing a valid created_on timestamp`);
      return { item, createdAt };
    })
    .sort((left, right) => right.createdAt - left.createdAt)[0].item;
}

function annotation(version, key) {
  return version?.annotations?.[key] ?? version?.metadata?.annotations?.[key] ?? "";
}

function commitFrom(version) {
  const tag = annotation(version, "workers/tag");
  const message = annotation(version, "workers/message");
  const match = /^github-ci candidate ([0-9a-f]{40})$/i.exec(message);
  if (!match || tag !== `git-${match[1].slice(0, 12)}`) {
    throw new Error("version provenance does not match a github-ci CI candidate commit");
  }
  return match[1].toLowerCase();
}

function bindingsOf(version) {
  const bindings = version?.bindings ?? version?.resources?.bindings;
  if (Array.isArray(bindings)) return bindings;
  const env = version?.env ?? version?.resources?.env;
  if (env && typeof env === "object" && !Array.isArray(env)) {
    return Object.entries(env).map(([name, value]) => ({ name, ...value }));
  }
  throw new Error("version response did not include its binding metadata");
}

function assertSecrets(version) {
  const bindings = bindingsOf(version);
  const byName = new Map(bindings.map((binding) => [binding.name, binding]));
  const missing = SECRET_BINDINGS.filter((name) => byName.get(name)?.type !== "secret_text");
  if (missing.length) {
    throw new Error(`version is missing required secret binding names or types: ${missing.join(", ")}`);
  }
}

function reportOutput(values, summary) {
  const outputPath = process.env.GITHUB_OUTPUT;
  if (!outputPath) throw new Error("GITHUB_OUTPUT is not set");
  appendFileSync(outputPath, Object.entries(values).map(([key, value]) => `${key}=${value}\n`).join(""));
  if (summary && process.env.GITHUB_STEP_SUMMARY) {
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${summary}\n`);
  }
}

function checkBuild(root) {
  const rootConfig = readJson(join(root, "config.json"));
  const worker = readJson(join(root, "workers/default/worker.config.json"));
  if (rootConfig.producer?.name !== "wrangler" || rootConfig.producer?.version !== "4.166.0") {
    throw new Error("Build Output must be produced by the pinned Wrangler 4.166.0 delegate");
  }
  if (worker.name !== "github-ci" || worker.compatibilityDate !== "2025-09-15" || worker.workersDev !== true) {
    throw new Error("Build Output worker identity, compatibility date, or workers.dev ingress differs from production");
  }
  if (Object.hasOwn(worker, "previewUrls")) {
    throw new Error("Build Output must preserve the existing preview URL policy without setting it");
  }
  for (const [name, value] of Object.entries(EXPECTED_TEXT_BINDINGS)) {
    const binding = worker.env?.[name];
    if (binding?.type !== "text" || binding.value !== value) {
      throw new Error(`Build Output non-secret binding mismatch: ${name}`);
    }
  }
  for (const name of SECRET_BINDINGS) {
    const binding = worker.env?.[name];
    if (binding?.type !== "secret" || Object.hasOwn(binding, "value")) {
      throw new Error(`Build Output must declare ${name} as a value-free secret binding`);
    }
  }
  console.log("Build Output v1 has the expected Worker, ingress, production vars, and value-free secret declarations.");
}

function selectUpload(path, sha) {
  if (!COMMIT_SHA.test(sha)) throw new Error("upload commit must be a full Git commit SHA");
  const tag = `git-${sha.slice(0, 12)}`;
  const versions = rows(readJson(path), "versions");
  const candidates = versions.filter((version) =>
    annotation(version, "workers/tag") === tag && annotation(version, "workers/message") === `github-ci candidate ${sha}`,
  );
  if (!candidates.length) throw new Error("uploaded version with exact commit tag and message was not listed");
  const version = newest(candidates, "candidate versions");
  if (!VERSION_ID.test(version.id ?? "")) throw new Error("selected upload did not have a Cloudflare Version UUID");
  reportOutput(
    { version_id: version.id },
    `Uploaded candidate \`${version.id}\` tagged \`${tag}\` from commit \`${sha}\`; production traffic was unchanged.`,
  );
}

function verifyVersion(path, requestedId, expectedSha) {
  if (!VERSION_ID.test(requestedId)) throw new Error("requested version ID is not a UUID");
  const version = unwrap(readJson(path));
  if (version?.id !== requestedId) throw new Error("Cloudflare returned a different Worker Version ID");
  const sha = commitFrom(version);
  if (expectedSha !== undefined && sha !== expectedSha.toLowerCase()) {
    throw new Error("version provenance does not match the requested source commit");
  }
  assertSecrets(version);
  const summary = `Verified candidate provenance \`${sha}\` and all ${SECRET_BINDINGS.length} retained secret binding names for version \`${requestedId}\`.`;
  reportOutput({ source_sha: sha }, summary);
}

function verifyDeployable(path, requestedId) {
  const versions = rows(readJson(path), "deployable versions");
  if (!versions.some((version) => version.id === requestedId)) {
    throw new Error("requested Worker Version is not in Cloudflare's deployable set");
  }
  console.log("Requested version is deployable.");
}

function verifyDeployment(path, requestedId, sha) {
  if (!COMMIT_SHA.test(sha)) throw new Error("deployment provenance must be a full Git commit SHA");
  const deployments = rows(readJson(path), "deployment read-back");
  const latest = newest(deployments, "deployment read-back");
  const versions = latest?.versions;
  if (!Array.isArray(versions) || versions.length !== 1 || versions[0].version_id !== requestedId || Number(versions[0].percentage) !== 100) {
    throw new Error("latest Cloudflare deployment does not attribute 100% traffic to the selected version");
  }
  reportOutput(
    { deployed_version_id: requestedId },
    `Read-back confirms version \`${requestedId}\` from commit \`${sha}\` is the latest deployment at 100% traffic.`,
  );
}

function main() {
  const parsed = parseArgs({
    args: process.argv.slice(2),
    options: {},
    allowPositionals: true,
    strict: true,
  });
  const [command, ...args] = parsed.positionals;
  if (command === "build" && args.length === 1) return checkBuild(args[0]);
  if (command === "select-upload" && args.length === 2) return selectUpload(args[0], args[1]);
  if (command === "verify-version" && (args.length === 2 || args.length === 3)) return verifyVersion(args[0], args[1], args[2]);
  if (command === "verify-deployable" && args.length === 2) return verifyDeployable(args[0], args[1]);
  if (command === "verify-deployment" && args.length === 3) return verifyDeployment(args[0], args[1], args[2]);
  usage();
}

try {
  main();
} catch (error) {
  console.error(`deployment guard: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 2;
}
