import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const WORKER_NAME = "decision-events";
const DATABASE_NAME = "decision-events";
const DATABASE_ID = "68ade85e-6835-46a8-af6a-a9f2f2d966ae";

function unwrap(payload) {
  let current = payload;
  for (let depth = 0; depth < 3 && current && typeof current === "object"; depth++) {
    if (current.result && typeof current.result === "object") current = current.result;
    else break;
  }
  return current;
}

export function assertTargetIdentity(workerPayload, databasePayload) {
  const worker = unwrap(workerPayload);
  const database = unwrap(databasePayload);
  assert.equal(worker?.name, WORKER_NAME, "Cloudflare Worker target name mismatch");
  assert.equal(database?.name, DATABASE_NAME, "Cloudflare D1 target name mismatch");
  assert.equal(
    database?.uuid ?? database?.id ?? database?.database_id,
    DATABASE_ID,
    "Cloudflare D1 database ID mismatch",
  );
}

async function main(args) {
  if (args.length !== 2) {
    console.error("usage: node scripts/check-target.mjs <worker-json> <d1-json>");
    process.exitCode = 2;
    return;
  }
  try {
    const [workerPath, databasePath] = args;
    const [workerJson, databaseJson] = await Promise.all([
      readFile(workerPath, "utf8").then(JSON.parse),
      readFile(databasePath, "utf8").then(JSON.parse),
    ]);
    assertTargetIdentity(workerJson, databaseJson);
    console.log("Cloudflare account contains the configured Worker and D1 database.");
  } catch (error) {
    console.error(`check-target: ${error.message}`);
    process.exitCode = 1;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  await main(process.argv.slice(2));
}
