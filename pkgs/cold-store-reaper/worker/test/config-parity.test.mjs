import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const root = dirname(here);
const wrangler = readFileSync(join(root, "wrangler.toml"), "utf8");
const cfConfig = readFileSync(join(root, "cloudflare.config.ts"), "utf8");

test("cf deployment config preserves the Worker identity and compatibility date", () => {
  assert.match(wrangler, /^name = "cold-store-rehydrator"$/m);
  assert.match(cfConfig, /name:\s*"cold-store-rehydrator"/);
  assert.match(wrangler, /^compatibility_date = "([^"]+)"$/m);
  const [, date] = wrangler.match(/^compatibility_date = "([^"]+)"$/m);
  assert.ok(cfConfig.includes(`compatibilityDate: "${date}"`));
  assert.match(cfConfig, /workersDev:\s*true/);
});

test("cf deployment config preserves the D1 binding and SQLite Durable Object migration", () => {
  const databaseId = wrangler.match(/^database_id = "([^"]+)"$/m)?.[1];
  const databaseName = wrangler.match(/^database_name = "([^"]+)"$/m)?.[1];
  assert.ok(databaseId && databaseName);
  assert.ok(cfConfig.includes(`id: "${databaseId}"`));
  assert.ok(cfConfig.includes(`name: "${databaseName}"`));
  assert.match(wrangler, /^binding = "MANIFEST"$/m);
  assert.match(cfConfig, /MANIFEST:\s*bindings\.d1\(/);

  assert.match(wrangler, /^name = "BUNDLE_FLIGHT"$/m);
  assert.match(wrangler, /^class_name = "BundleFlight"$/m);
  assert.match(wrangler, /new_sqlite_classes\s*=\s*\["BundleFlight"\]/);
  assert.match(cfConfig, /BUNDLE_FLIGHT:\s*bindings\.durableObject\(/);
  assert.match(cfConfig, /BundleFlight:\s*exports\.durableObject\(\{\s*storage:\s*"sqlite"\s*\}\)/);
});

test("cf deployment config retains all current secret binding names", () => {
  for (const name of ["REHYDRATOR_API_KEY", "GOOGLE_CLIENT_ID", "GOOGLE_CLIENT_SECRET", "GOOGLE_REFRESH_TOKEN"]) {
    assert.match(wrangler, new RegExp(`^#?\\s*${name}\\b`, "m"));
    assert.ok(cfConfig.includes(`${name}: bindings.secret()`));
  }
});
