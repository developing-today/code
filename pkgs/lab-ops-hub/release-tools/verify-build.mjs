import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const packageDir = dirname(dirname(fileURLToPath(import.meta.url)));
const readJsonc = (path) =>
  JSON.parse(readFileSync(join(packageDir, path), "utf8").replace(/^\s*\/\/.*$/gm, ""));
const wrangler = readJsonc("wrangler.jsonc");
const outputPath = join(packageDir, ".cloudflare/output/v1/workers/default/worker.config.json");
assert.ok(existsSync(outputPath), "cf build must produce the v1 default Worker config");
const output = JSON.parse(readFileSync(outputPath, "utf8"));

assert.equal(wrangler.name, "lab-ops-hub");
assert.equal(wrangler.main, "src/index.ts");
assert.equal(wrangler.routes, undefined);
assert.equal(wrangler.workers_dev, undefined);
assert.deepEqual(wrangler.assets, {
  directory: "./public",
  binding: "ASSETS",
  run_worker_first: ["/api/*"],
});
assert.equal(output.name, wrangler.name);
assert.equal(output.compatibilityDate, wrangler.compatibility_date);
assert.deepEqual(output.compatibilityFlags, wrangler.compatibility_flags);
assert.deepEqual(output.observability, wrangler.observability);
assert.deepEqual(output.assets, { runWorkerFirst: wrangler.assets.run_worker_first });
assert.equal(output.manifest.type, "complete");
assert.equal(output.manifest.mainModule, "index.js");
assert.equal(output.routes, undefined);
assert.equal(output.workersDev, undefined);
const filesUnder = (directory, prefix = "") =>
  readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const relative = join(prefix, entry.name);
    return entry.isDirectory()
      ? filesUnder(join(directory, entry.name), relative)
      : [relative];
  }).sort();
assert.deepEqual(
  filesUnder(join(packageDir, "public")),
  filesUnder(join(packageDir, ".cloudflare/output/v1/workers/default/assets")),
  "all public assets must be carried into Build Output",
);
assert.match(readFileSync(join(packageDir, "wrangler.config.ts"), "utf8"), /assetsDirectory:\s*"\.\/public"/);

assert.deepEqual(wrangler.migrations, [
  { tag: "v1", new_sqlite_classes: ["LiveRoom", "CrawlerContainer"] },
  { tag: "v2", new_sqlite_classes: ["CrawlerContainerV2"] },
]);
assert.deepEqual(wrangler.durable_objects.bindings, [
  { name: "LIVE", class_name: "LiveRoom" },
  { name: "CRAWLER", class_name: "CrawlerContainerV2" },
]);
assert.deepEqual(wrangler.d1_databases, [
  { binding: "DB", database_name: "labops-db", database_id: "3115e67f-150e-4ee8-8b11-216127901ae8" },
]);
assert.deepEqual(wrangler.r2_buckets, [{ binding: "FILES", bucket_name: "labops-files" }]);
assert.deepEqual(wrangler.kv_namespaces, [{ binding: "CACHE", id: "ac69d54d9e3f455d99a940b3fced4b76" }]);
assert.deepEqual(wrangler.queues, {
  producers: [{ binding: "JOBS", queue: "labops-jobs" }],
  consumers: [{ queue: "labops-jobs", max_batch_size: 10, max_retries: 3 }],
});
assert.deepEqual(wrangler.ai, { binding: "AI" });
assert.deepEqual(wrangler.triggers, { crons: ["23 4 * * *"] });
assert.deepEqual(output.env.LIVE, {
  type: "durable-object",
  worker: wrangler.name,
  exportName: "LiveRoom",
});
assert.deepEqual(output.env.CRAWLER, {
  type: "durable-object",
  worker: wrangler.name,
  exportName: "CrawlerContainerV2",
});

const d1 = wrangler.d1_databases[0];
assert.deepEqual(output.env.DB, { type: "d1", name: d1.database_name, id: d1.database_id });
const r2 = wrangler.r2_buckets[0];
assert.deepEqual(output.env.FILES, { type: "r2", name: r2.bucket_name });
const kv = wrangler.kv_namespaces[0];
assert.deepEqual(output.env.CACHE, { type: "kv", id: kv.id });
const producer = wrangler.queues.producers[0];
assert.deepEqual(output.env.JOBS, { type: "queue", name: producer.queue });
assert.deepEqual(output.env.AI, { type: "ai" });
assert.deepEqual(output.env.ASSETS, { type: "assets" });

const consumer = wrangler.queues.consumers[0];
assert.deepEqual(output.triggers, [
  { type: "scheduled", schedule: wrangler.triggers.crons[0] },
  {
    type: "queue",
    name: consumer.queue,
    maxBatchSize: consumer.max_batch_size,
    maxRetries: consumer.max_retries,
  },
]);

assert.deepEqual(output.env, {
  DB: { type: "d1", name: "labops-db", id: "3115e67f-150e-4ee8-8b11-216127901ae8" },
  FILES: { type: "r2", name: "labops-files" },
  CACHE: { type: "kv", id: "ac69d54d9e3f455d99a940b3fced4b76" },
  JOBS: { type: "queue", name: "labops-jobs" },
  LIVE: { type: "durable-object", worker: "lab-ops-hub", exportName: "LiveRoom" },
  CRAWLER: { type: "durable-object", worker: "lab-ops-hub", exportName: "CrawlerContainerV2" },
  AI: { type: "ai" },
  ASSETS: { type: "assets" },
});

const container = wrangler.containers[0];
assert.equal(container.class_name, "CrawlerContainerV2");
assert.equal(container.image, "registry.cloudflare.com/234b866b99ce2b142d7963d8d139cbfa/lab-ops-hub-crawlercontainer:v5");
assert.match(readFileSync(join(packageDir, "container-image.nix"), "utf8"), /tag\s*=\s*"v5";/);
assert.match(readFileSync(join(packageDir, "image-push.mjs"), "utf8"), /const IMAGE = "lab-ops-hub-crawlercontainer:v5";/);
assert.deepEqual(output.unsafe.metadata.containers, [container]);

const releaseTools = JSON.parse(readFileSync(join(packageDir, "release-tools/package.json"), "utf8"));
assert.equal(releaseTools.dependencies.cf, "1.0.0-rc.1");
assert.equal(releaseTools.dependencies.wrangler, "4.163.0");
assert.equal(releaseTools.engines.node, "24.22.x");

console.log("Lab Ops Hub Build Output v1 matches Wrangler target, bindings, migrations, triggers, assets, and pinned container image");
