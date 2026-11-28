import assert from "node:assert/strict";
import { test } from "node:test";
import { handle, versionAnswer } from "../src/http.js";

// Oracle: #15793 criterion 1 — every in-service Worker names the commit
// it was deployed from on unauthenticated GET /version, in the shape
// the edge probe reads (commitOf in
// pm-stack/console/edge/src/worker-checks.ts takes the `version` field).

const req = (path, method = "GET") => new Request(`https://d1-gateway.internal${path}`, { method });

test("versionAnswer names the stamped commit", () => {
  assert.deepEqual(versionAnswer({ VERSION: "abc123" }), {
    ok: true,
    service: "d1-gateway",
    version: "abc123",
  });
});

test("versionAnswer is honest when unstamped", () => {
  assert.deepEqual(versionAnswer({}), { ok: true, service: "d1-gateway", version: "unknown" });
});

test("GET /version answers without auth and touches no database", async () => {
  const res = await handle(req("/version"), { VERSION: "abc123" });
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), { ok: true, service: "d1-gateway", version: "abc123" });
});

test("POST /version is a 405", async () => {
  const res = await handle(req("/version", "POST"), { VERSION: "abc123" });
  assert.equal(res.status, 405);
});

test("/health answer is unchanged", async () => {
  const res = await handle(req("/health"), {});
  assert.equal(res.status, 200);
  const body = await res.json();
  assert.equal(body.ok, true);
  assert.equal(body.service, "d1-gateway");
  assert.deepEqual(body.databases, []);
});

test("unknown paths still 404", async () => {
  const res = await handle(req("/nope"), {});
  assert.equal(res.status, 404);
});
