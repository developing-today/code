// #3400: the raw USERS namespace binding is owned exclusively by the
// routing module (src/routing.ts). No application path may bypass it:
// object IDs and stubs must come from the SAME jurisdiction-restricted
// namespace via userStub(). This static test enforces that boundary so a
// future raw-binding bypass fails loudly here instead of silently
// producing a second, empty user store for a missing jurisdiction.
//
// Allowed touches:
// - src/routing.ts: the module itself.
// - src/worker.ts: the front door, which passes env.USERS into
//   userStub() but never derives IDs or stubs itself.
// - src/test-harness.ts: the test-only entry point (wrangler.test.toml);
//   it self-declares TEST-ONLY and the app wrangler.toml must not
//   reference it.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const testDir = fileURLToPath(new URL(".", import.meta.url));
const srcDir = join(testDir, "..", "src");
const pkgDir = join(testDir, "..");

function collectTs(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir).sort()) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      collectTs(full, out);
    } else if (entry.endsWith(".ts")) {
      out.push(full);
    }
  }
  return out;
}

function filesContaining(pattern: RegExp): string[] {
  return collectTs(srcDir)
    .filter((full) => pattern.test(readFileSync(full, "utf8")))
    .map((full) => full.slice(srcDir.length + 1).replace(/\\/g, "/"));
}

test("namespace derivation exists only in the routing module and the test harness", () => {
  assert.deepEqual(filesContaining(/idFromName/), ["routing.ts", "test-harness.ts"]);
});

test("jurisdiction restriction is selected only in the routing module and the test harness", () => {
  assert.deepEqual(filesContaining(/\.jurisdiction\(/), ["routing.ts", "test-harness.ts"]);
});

test("the raw USERS binding is referenced only by the routing module and the front door", () => {
  assert.deepEqual(filesContaining(/\bUSERS\b/), ["routing.ts", "worker.ts"]);
});

test("the front door never derives IDs or stubs itself", () => {
  const worker = readFileSync(join(srcDir, "worker.ts"), "utf8");
  assert.ok(!/idFromName/.test(worker), "worker.ts must not call idFromName");
  assert.ok(!/\.jurisdiction\(/.test(worker), "worker.ts must not select a jurisdiction namespace");
  assert.ok(worker.includes("userStub(env.USERS,"), "worker.ts must route through userStub(env.USERS, ...)");
});

test("the test harness stays test-only", () => {
  const harness = readFileSync(join(srcDir, "test-harness.ts"), "utf8");
  assert.ok(harness.includes("TEST-ONLY"), "test-harness.ts must keep its TEST-ONLY declaration");
  const appConfig = readFileSync(join(pkgDir, "wrangler.toml"), "utf8");
  assert.ok(!appConfig.includes("test-harness"), "the app wrangler.toml must not reference the test harness");
});
