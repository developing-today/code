import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { versionAnswer } from "../src/version.js";

// Oracle: #15793 criterion 1 (parent #13494) — every in-service Worker
// answers unauthenticated GET /version with the commit it was deployed
// from, in the `version`-field shape the edge probe reads (commitOf in
// pm-stack/console/edge/src/worker-checks.ts); "unknown", never a
// fabricated commit, when the deploy was unstamped. The fetch-handler
// wiring is pinned textually here because src/index.js imports
// cloudflare:workers, which only exists inside workerd; the helper it
// answers with is unit-tested directly.

test("versionAnswer names the stamped commit and trims it", () => {
  assert.deepEqual(versionAnswer({ VERSION: "abc123" }), { ok: true, service: "queue-transport-alert", version: "abc123" });
  assert.deepEqual(versionAnswer({ VERSION: "  abc123  " }), { ok: true, service: "queue-transport-alert", version: "abc123" });
});

test("versionAnswer is honest when unstamped or blank", () => {
  assert.deepEqual(versionAnswer({}), { ok: true, service: "queue-transport-alert", version: "unknown" });
  assert.deepEqual(versionAnswer({ VERSION: "   " }), { ok: true, service: "queue-transport-alert", version: "unknown" });
  assert.deepEqual(versionAnswer(undefined), { ok: true, service: "queue-transport-alert", version: "unknown" });
});

test("the fetch handler answers /version ahead of the POST gate", () => {
  const src = readFileSync(new URL("../src/index.js", import.meta.url), "utf8");
  const versionAt = src.indexOf('pathname === "/version"');
  const postGateAt = src.indexOf('if (request.method === "POST")');
  assert.ok(versionAt !== -1, "index.js routes /version");
  assert.ok(versionAt < postGateAt, "/version is answered before the POST handling");
  assert.match(src, /if \(request\.method !== "GET"\) return new Response\("method not allowed/);
  assert.match(src, /return Response\.json\(versionAnswer\(env\)\);/);
});
