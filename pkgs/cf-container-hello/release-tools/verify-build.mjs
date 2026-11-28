// Compare cf's effective Build Output against the preserved Wrangler source.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const wrangler = JSON.parse(readFileSync(new URL("../wrangler.jsonc", import.meta.url), "utf8"));
const output = JSON.parse(readFileSync(new URL("../.cloudflare/output/v1/workers/default/worker.config.json", import.meta.url), "utf8"));

assert.equal(output.name, wrangler.name);
assert.equal(output.compatibilityDate, wrangler.compatibility_date);
assert.deepEqual(output.observability, wrangler.observability);
assert.deepEqual(output.env, {
  HELLO_CONTAINER: { type: "durable-object", worker: wrangler.name, exportName: "HelloContainer" },
  CRAWLER_PROBE: { type: "durable-object", worker: wrangler.name, exportName: "CrawlerProbe" },
  HELLO_LITE: { type: "durable-object", worker: wrangler.name, exportName: "HelloLite" },
});
assert.deepEqual(output.unsafe?.metadata?.containers, wrangler.containers);
assert.deepEqual(output.unsafe?.metadata?.migrations, wrangler.migrations);
const serialized = JSON.stringify(output);
for (const name of ["HelloContainer", "CrawlerProbe", "HelloLite"]) {
  assert.ok(serialized.includes(name), `Build Output must include ${name}`);
}
assert.ok(serialized.includes("lab-ops-hub-crawlercontainer:v5"), "Build Output must preserve CrawlerProbe :v5 image");
assert.ok(serialized.includes("new_sqlite_classes"), "Build Output must preserve SQLite migration groups");

console.log("cf Build Output matches Worker identity, bindings, container refs, and both SQLite migration groups");
