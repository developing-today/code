// Usage: node scripts/check-health.mjs --url HTTPS_URL --version FULL_SHA
// Strictly accepts those two flags once each; rejects any other argument.
// The deploy workflow's health gate (design §4): the promoted
// version's /health must report the exact commit and a truthful age
// before the deploy counts as landed. Shape follows the
// decision-events Worker's check-health.mjs.
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export function matchesHealth(payload, expectedVersion, nowMs = Date.now()) {
  if (payload?.ok !== true || payload?.version !== expectedVersion) return false;
  if (typeof payload.deployedAt !== "string" || !Number.isInteger(payload.ageSeconds) || payload.ageSeconds < 0) return false;

  const deployedAtMs = Date.parse(payload.deployedAt);
  if (!Number.isFinite(deployedAtMs) || deployedAtMs > nowMs) return false;
  const expectedAgeSeconds = Math.floor((nowMs - deployedAtMs) / 1_000);
  return Math.abs(payload.ageSeconds - expectedAgeSeconds) <= 5;
}

function parseArgs(args) {
  const values = new Map();
  for (let i = 0; i < args.length; i += 1) {
    const key = args[i];
    if (key !== "--url" && key !== "--version") {
      throw new Error(`unexpected argument: ${key}`);
    }
    if (values.has(key) || !args[i + 1] || args[i + 1].startsWith("--")) {
      throw new Error(`expected one value after ${key}`);
    }
    values.set(key, args[++i]);
  }
  const url = values.get("--url");
  const version = values.get("--version");
  if (values.size !== 2 || !url || !version) {
    throw new Error("usage: node scripts/check-health.mjs --url HTTPS_URL --version FULL_SHA");
  }
  const parsedUrl = new URL(url);
  if (parsedUrl.protocol !== "https:") {
    throw new Error("--url must use HTTPS");
  }
  if (!/^[0-9a-f]{40}$/.test(version)) {
    throw new Error("--version must be a full 40-character lowercase commit SHA");
  }
  return { url: parsedUrl, version };
}

async function waitForHealthyVersion(url, expectedVersion) {
  let lastAnswer = "no response";
  for (let attempt = 1; attempt <= 20; attempt += 1) {
    try {
      const response = await fetch(url, { signal: AbortSignal.timeout(10_000) });
      const payload = await response.json();
      lastAnswer = `HTTP ${response.status}: ${JSON.stringify(payload)}`;
      if (response.ok && matchesHealth(payload, expectedVersion)) {
        console.log(`pm-service answers with version ${expectedVersion}`);
        return;
      }
    } catch (error) {
      lastAnswer = error instanceof Error ? error.message : String(error);
    }
    if (attempt < 20) await new Promise((resolveSleep) => setTimeout(resolveSleep, 6_000));
  }
  throw new Error(`health check did not report ${expectedVersion}; last answer: ${lastAnswer}`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const { url, version } = parseArgs(process.argv.slice(2));
    await waitForHealthyVersion(url, version);
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
