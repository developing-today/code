import test from "node:test";
import assert from "node:assert/strict";
import worker from "./src/index.js";
import { versionAnswer } from "./src/version.js";

// Oracle: #15793 criterion 1 (parent #13494) — every in-service Worker
// answers unauthenticated GET /version with the commit it was deployed
// from, in the `version`-field shape the edge probe reads (commitOf in
// pm-stack/console/edge/src/worker-checks.ts); "unknown", never a
// fabricated commit, when the deploy was unstamped.

const req = (path, method = "GET") => new Request(`https://muse-agent-handoff.test${path}`, { method });

test("versionAnswer names the stamped commit and trims it", () => {
  assert.deepEqual(versionAnswer({ VERSION: "abc123" }), { ok: true, service: "muse-agent-handoff", version: "abc123" });
  assert.deepEqual(versionAnswer({ VERSION: "  abc123  " }), { ok: true, service: "muse-agent-handoff", version: "abc123" });
});

test("versionAnswer is honest when unstamped or blank", () => {
  assert.deepEqual(versionAnswer({}), { ok: true, service: "muse-agent-handoff", version: "unknown" });
  assert.deepEqual(versionAnswer({ VERSION: "   " }), { ok: true, service: "muse-agent-handoff", version: "unknown" });
  assert.deepEqual(versionAnswer(undefined), { ok: true, service: "muse-agent-handoff", version: "unknown" });
});

test("GET /version answers without auth and touches no binding", async () => {
  const res = await worker.fetch(req("/version"), { VERSION: "abc123" });
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), { ok: true, service: "muse-agent-handoff", version: "abc123" });
});

test("GET /version answers unknown when unstamped", async () => {
  const res = await worker.fetch(req("/version"), {});
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), { ok: true, service: "muse-agent-handoff", version: "unknown" });
});

test("POST /version is a 405, and the existing routes are untouched", async () => {
  const post = await worker.fetch(req("/version", "POST"), { VERSION: "abc123" });
  assert.equal(post.status, 405);
  const root = await worker.fetch(req("/"), {});
  assert.equal(root.status, 200);
  assert.match(await root.text(), /Muse Fleet handoff service/);
  const missing = await worker.fetch(req("/nope"), {});
  assert.equal(missing.status, 404);
});
