// Offline validation helpers for the GHA cf Versions workflow.
// This module never performs Cloudflare requests or deployments.
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

export const KV_NAMESPACE_ID = "46b1fe46e6434c5fa509556ed3a53ea2";
export const REQUIRED_SECRET_BINDINGS = ["DRAIN_BEARER", "GITHUB_WEBHOOK_SECRET"];

const VERSION_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

function readJson(path) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    throw new Error(`cannot read Cloudflare JSON response at ${path}: ${error.message}`);
  }
}

function responseObject(payload) {
  return payload?.result ?? payload;
}

function versionId(payload) {
  const id = responseObject(payload)?.id;
  if (typeof id !== "string" || !VERSION_ID.test(id)) {
    throw new Error("Cloudflare response did not contain a full Worker version UUID");
  }
  return id;
}

export function uploadedVersionId(payload) {
  return versionId(payload);
}

export function assertConfiguredVersion(payload, requestedId) {
  if (!VERSION_ID.test(requestedId)) {
    throw new Error("promotion requires a full Worker version UUID");
  }
  const version = responseObject(payload);
  if (versionId(payload) !== requestedId) {
    throw new Error(`Cloudflare returned a different Worker version than requested (${requestedId})`);
  }

  const bindings = [
    ...(Array.isArray(version.bindings) ? version.bindings : []),
    ...(Array.isArray(version.resources?.bindings) ? version.resources.bindings : []),
  ];
  for (const name of REQUIRED_SECRET_BINDINGS) {
    if (!bindings.some((binding) => binding?.name === name && binding.type === "secret_text")) {
      throw new Error(`version ${requestedId} is missing required secret binding ${name}`);
    }
  }
  if (!bindings.some((binding) =>
    binding?.name === "GHB_EVENTS" &&
    binding.type === "kv_namespace" &&
    binding.namespace_id === KV_NAMESPACE_ID
  )) {
    throw new Error(`version ${requestedId} does not retain the configured GHB_EVENTS KV namespace`);
  }
}

export function assertDeployment(payload, requestedId) {
  if (!VERSION_ID.test(requestedId)) {
    throw new Error("promotion verification requires a full Worker version UUID");
  }
  const unwrapped = responseObject(payload);
  const deployments = Array.isArray(unwrapped)
    ? unwrapped
    : Array.isArray(unwrapped?.items)
      ? unwrapped.items
      : [];
  if (deployments.length === 0) {
    throw new Error("Cloudflare returned no Worker deployments after promotion");
  }

  const latest = deployments.reduce((current, candidate) => {
    const currentDate = Date.parse(current.created_on ?? "");
    const candidateDate = Date.parse(candidate.created_on ?? "");
    if (!Number.isFinite(currentDate) || !Number.isFinite(candidateDate)) {
      throw new Error("Cloudflare deployment response is missing a valid created_on timestamp");
    }
    return candidateDate > currentDate ? candidate : current;
  });
  const live = (latest.versions ?? []).filter((version) => version.percentage === 100);
  if (live.length !== 1 || live[0].version_id !== requestedId) {
    throw new Error(`latest Worker deployment is not exactly ${requestedId} at 100%`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [, , mode, path, expectedId] = process.argv;
  const payload = readJson(path);
  if (mode === "upload-result") {
    process.stdout.write(`${uploadedVersionId(payload)}\n`);
  } else if (mode === "verify-version") {
    assertConfiguredVersion(payload, expectedId);
    process.stdout.write(`verified version ${expectedId} has required secrets and the original KV namespace\n`);
  } else if (mode === "verify-deployment") {
    assertDeployment(payload, expectedId);
    process.stdout.write(`verified version ${expectedId} is the latest 100% deployment\n`);
  } else {
    throw new Error("usage: release-version.mjs upload-result|verify-version|verify-deployment <json-path> [version-id]");
  }
}
