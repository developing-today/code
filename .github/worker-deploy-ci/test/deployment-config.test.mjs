import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import test from "node:test";

const read = (path) => readFile(new URL(path, import.meta.url), "utf8");
const SECRET_BINDINGS = [
  "GITHUB_WEBHOOK_SECRET",
  "TURSO_PLATFORM_TOKEN",
  "TURSO_QUEUE_TOKEN",
  "EVENTS_BROKER_TOKEN",
  "EVENTS_BROKER_ACCESS_CLIENT_ID",
  "EVENTS_BROKER_ACCESS_CLIENT_SECRET",
];

test("cf config declares the production non-secret bindings and only secret names", async () => {
  const config = await read("../cloudflare.config.ts");
  const wrangler = await read("../../../pkgs/github-ci/worker/wrangler.toml");
  assert.match(config, /name:\s*"github-ci"/);
  assert.match(config, /compatibilityDate:\s*"2025-09-15"/);
  assert.match(config, /workersDev:\s*true/);
  assert.match(config, /\.\.\/\.\.\/pkgs\/github-ci\/worker\/src\/index\.ts/);
  assert.doesNotMatch(config, /previewUrls\s*:/);
  for (const [name, value] of [
    ["TURSO_PLATFORM_URL", "https://fleet-platform-dezren39.aws-us-east-1.turso.io"],
    ["TURSO_QUEUE_URL", "https://fleet-queue-dezren39.aws-us-east-1.turso.io"],
    ["EVENTS_BROKER_URL", "https://queue-broker.security.cab"],
  ]) {
    assert.match(wrangler, new RegExp(`${name}\\s*=\\s*"${value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}"`));
    assert.match(config, new RegExp(`${name}:\\s*bindings\\.text\\("${value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}"\\)`));
  }
  for (const name of SECRET_BINDINGS) assert.match(config, new RegExp(`${name}:\\s*bindings\\.secret\\(\\)`));
  assert.doesNotMatch(config, /bindings\.secret\([^)]/);
});

test("Build Output v1 was made by pinned Wrangler and preserves value-free secret names", async () => {
  const root = JSON.parse(await read("../.cloudflare/output/v1/config.json"));
  const worker = JSON.parse(await read("../.cloudflare/output/v1/workers/default/worker.config.json"));
  assert.deepEqual(root.producer, { name: "wrangler", version: "4.166.0" });
  assert.equal(worker.name, "github-ci");
  assert.equal(worker.workersDev, true);
  assert.equal(Object.hasOwn(worker, "previewUrls"), false);
  for (const name of SECRET_BINDINGS) {
    assert.deepEqual(worker.env[name], { type: "secret" });
  }
});

test("locked toolchain uses the cf release candidate with matching Build Output v1", async () => {
  const pkg = JSON.parse(await read("../package.json"));
  const lock = JSON.parse(await read("../package-lock.json"));
  const workerPkg = JSON.parse(await read("../../../pkgs/github-ci/worker/package.json"));
  const workerLock = JSON.parse(await read("../../../pkgs/github-ci/worker/package-lock.json"));
  assert.equal(pkg.engines.node, "24.22.0");
  assert.equal(pkg.devDependencies.cf, "1.0.0-rc.1");
  assert.equal(lock.packages["node_modules/cf"].version, pkg.devDependencies.cf);
  assert.equal(pkg.devDependencies.wrangler, "4.166.0");
  assert.equal(lock.packages["node_modules/wrangler"].version, pkg.devDependencies.wrangler);
  assert.equal(lock.packages["node_modules/@cloudflare/build-output-utils"].version, "1.0.0-rc.1");
  assert.equal(workerPkg.scripts.deploy, "wrangler deploy");
  assert.equal(workerPkg.devDependencies.wrangler, "4.59.1");
  assert.equal(Object.hasOwn(workerPkg.devDependencies, "cf"), false);
  assert.equal(workerLock.packages["node_modules/wrangler"].version, "4.59.1");
});

test("pinned cf Versions upload requires strict inheritance resolution", async () => {
  const distUrl = new URL("../node_modules/cf/dist/", import.meta.url);
  const files = await readdir(distUrl);
  const uploadModules = files.filter((file) => file.startsWith("static-assets-") && file.endsWith(".mjs"));
  assert.ok(uploadModules.length > 0, "cf upload implementation module is present");
  const source = (await Promise.all(uploadModules.map((file) => readFile(new URL(file, distUrl), "utf8")))).join("\n");
  assert.match(source, /bindings_inherit:\s*`strict`/);
});
