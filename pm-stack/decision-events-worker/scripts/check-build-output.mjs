import assert from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";

const OUTPUT_ROOT = new URL("../.cloudflare/output/", import.meta.url);
const DATABASE_ID = "68ade85e-6835-46a8-af6a-a9f2f2d966ae";

const versions = (await readdir(OUTPUT_ROOT, { withFileTypes: true }))
  .filter((entry) => entry.isDirectory() && /^v\d+$/.test(entry.name))
  .map((entry) => entry.name)
  .sort((a, b) => Number(b.slice(1)) - Number(a.slice(1)));
assert.ok(versions.length > 0, "cf Build Output directory is missing");

const configPath = new URL(`../.cloudflare/output/${versions[0]}/workers/default/worker.config.json`, import.meta.url);
const config = JSON.parse(await readFile(configPath, "utf8"));
assert.equal(config.name, "decision-events");
assert.equal(config.compatibilityDate, "2025-09-15");
assert.equal(config.workersDev, true);
assert.deepEqual(config.env?.DB, {
  type: "d1",
  name: "decision-events",
  id: DATABASE_ID,
});
assert.deepEqual(config.env?.SET_EVENTS, {
  type: "durable-object",
  worker: "decision-events",
  exportName: "DecisionSetEvents",
});
assert.deepEqual(config.exports?.DecisionSetEvents, {
  type: "durable-object",
  storage: "sqlite",
});
assert.equal("domains" in config, false, "this release must not add custom domains");
assert.equal("triggers" in config, false, "this release must not add routes or triggers");

// The Build Output config carries DO class/storage but not Wrangler's
// migration tag. Pin and verify the unchanged pre-existing v1 migration
// declaration here so a change to the source deployment config is visible.
const wrangler = await readFile(new URL("../wrangler.toml", import.meta.url), "utf8");
assert.match(wrangler, /database_id\s*=\s*"68ade85e-6835-46a8-af6a-a9f2f2d966ae"/);
assert.match(wrangler, /binding\s*=\s*"DB"/);
assert.match(wrangler, /class_name\s*=\s*"DecisionSetEvents"/);
assert.match(wrangler, /tag\s*=\s*"v1"\s*\n\s*new_sqlite_classes\s*=\s*\["DecisionSetEvents"\]/);

console.log("Build Output matches the pinned D1, Durable Object, and routing settings.");
