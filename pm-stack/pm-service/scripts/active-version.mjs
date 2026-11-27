// Usage: node scripts/active-version.mjs --file DEPLOYMENT_STATUS_JSON
// Strictly accepts --file once with one path value. Reads Wrangler's JSON
// status only after argument validation and prints the sole version serving
// 100% of traffic; missing or gradual deployments fail closed before upload.
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const VERSION_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

export function activeVersionId(deployment) {
  const versions = deployment?.versions;
  if (!Array.isArray(versions) || versions.length !== 1) {
    throw new Error("expected one active pm-service deployment version");
  }
  const [version] = versions;
  if (version.percentage !== 100) {
    throw new Error("expected pm-service deployment version to serve 100% of traffic");
  }
  if (typeof version.version_id !== "string" || !VERSION_ID.test(version.version_id)) {
    throw new Error("pm-service deployment has an invalid version_id");
  }
  return version.version_id;
}

function parseArgs(args) {
  if (args.length !== 2 || args[0] !== "--file" || !args[1] || args[1].startsWith("--")) {
    throw new Error("usage: node scripts/active-version.mjs --file DEPLOYMENT_STATUS_JSON");
  }
  return { file: args[1] };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const { file } = parseArgs(process.argv.slice(2));
    const deployment = JSON.parse(await readFile(file, "utf8"));
    process.stdout.write(`${activeVersionId(deployment)}\n`);
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 2;
  }
}
