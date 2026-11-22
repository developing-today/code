import assert from "node:assert/strict";
import { lstat, readdir, readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

if (process.argv.length !== 2) { console.error("Usage: node scripts/validate-artifact.mjs"); process.exit(2); }

const projectRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));
const workerPath = resolve(projectRoot, "dist/server/index.js");
const manifestPath = resolve(projectRoot, "dist/.openai/hosting.json");

async function checkExact(path, expected) {
  const st = await lstat(path);
  if (expected === null) {
    assert(st.isFile() && st.nlink === 1, "Artifact file must be regular: " + path);
    return;
  }
  assert(st.isDirectory(), "Artifact directory must be real: " + path);
  assert.deepEqual((await readdir(path)).sort(), Object.keys(expected).sort(), "Unexpected artifact inventory: " + path);
  for (const [name, shape] of Object.entries(expected)) await checkExact(resolve(path, name), shape);
}
await checkExact(resolve(projectRoot, "dist"), {server:{"index.js":null}, ".openai":{"hosting.json":null}});

const [source, manifest] = await Promise.all([
  readFile(workerPath, "utf8"),
  readFile(manifestPath, "utf8"),
]);
JSON.parse(manifest);

// A data URL forces ESM parsing even though the generated output has no package.json.
const moduleUrl = `data:text/javascript;base64,${Buffer.from(source).toString("base64")}`;
const workerModule = await import(moduleUrl);
assert.equal(
  typeof workerModule.default?.fetch,
  "function",
  `${pathToFileURL(workerPath)} must export default.fetch`,
);

console.log("Artifact is valid ESM and exports default.fetch");
