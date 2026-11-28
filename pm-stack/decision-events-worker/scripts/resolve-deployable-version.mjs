import assert from "node:assert/strict";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const WORKER = "decision-events";
const API_ORIGIN = "https://api.cloudflare.com/client/v4";

export function selectTaggedDeployableVersion(payload, tag) {
  const versions = Array.isArray(payload?.result) ? payload.result : payload?.result?.items;
  if (payload?.success !== true || !Array.isArray(versions)) {
    throw new Error("Cloudflare did not return a successful Worker versions list");
  }
  const candidates = versions.filter(
    (version) => version?.annotations?.["workers/tag"] === tag && typeof version.id === "string",
  );
  candidates.sort((a, b) => String(b.created_on ?? "").localeCompare(String(a.created_on ?? "")));
  if (!candidates.length) throw new Error(`no deployable version found with tag ${tag}`);
  return candidates[0].id;
}

function parseArgs(args) {
  if (args.length !== 1 || !/^git-[0-9a-f]{12}$/.test(args[0])) {
    throw new Error("usage: node scripts/resolve-deployable-version.mjs git-<sha12>");
  }
  return args[0];
}

export async function resolveDeployableVersion({ tag, accountId, apiToken, fetchImpl = fetch }) {
  const url = new URL(`${API_ORIGIN}/accounts/${accountId}/workers/scripts/${WORKER}/versions`);
  url.searchParams.set("deployable", "true");
  url.searchParams.set("per_page", "100");
  const response = await fetchImpl(url, {
    headers: { Authorization: `Bearer ${apiToken}` },
  });
  const payload = await response.json().catch(() => null);
  if (!response.ok) {
    throw new Error(`Cloudflare versions query failed with HTTP ${response.status}`);
  }
  return selectTaggedDeployableVersion(payload, tag);
}

async function main() {
  // Validate the complete CLI contract before reading credentials or making
  // a network request. This utility only reads version metadata.
  const tag = parseArgs(process.argv.slice(2));
  const accountId = process.env.CLOUDFLARE_ACCOUNT_ID;
  const apiToken = process.env.CLOUDFLARE_API_TOKEN;
  assert.match(accountId ?? "", /^[0-9a-f]{32}$/i, "CLOUDFLARE_ACCOUNT_ID must be a 32-digit account ID");
  assert.ok(apiToken, "CLOUDFLARE_API_TOKEN is required");
  const versionId = await resolveDeployableVersion({ tag, accountId, apiToken });
  process.stdout.write(`${versionId}\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch((error) => {
    console.error(`resolve-deployable-version: ${error.message}`);
    process.exitCode = 1;
  });
}
