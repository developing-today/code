import assert from "node:assert/strict";
import { test } from "node:test";
import { route } from "../src/routes.js";
import { versionAnswer } from "../src/version.js";

// Oracle: #15793 criterion 1 — every in-service Worker names the commit
// it was deployed from on unauthenticated GET /version, in the shape
// the edge probe reads (commitOf in
// pm-stack/console/edge/src/worker-checks.ts takes the `version` field).

const BASE = "https://bridge.example";
const noStore = () => {
  throw new Error("/version must not touch the store");
};
const call = (path, env, init) => route(new Request(BASE + path, init), env, noStore);

test("versionAnswer names the stamped commit", () => {
  assert.deepEqual(versionAnswer({ VERSION: "abc123" }), { ok: true, service: "message-bridge", version: "abc123" });
});

test("versionAnswer is honest when unstamped", () => {
  assert.deepEqual(versionAnswer({}), { ok: true, service: "message-bridge", version: "unknown" });
});

test("GET /version answers without auth and touches no store", async () => {
  const res = await call("/version", { VERSION: "abc123" });
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), { ok: true, service: "message-bridge", version: "abc123" });
});

test("GET /version answers unknown when unstamped", async () => {
  const res = await call("/version", {});
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), { ok: true, service: "message-bridge", version: "unknown" });
});

test("POST /version is a 405", async () => {
  const res = await call("/version", { VERSION: "abc123" }, { method: "POST", body: "{}" });
  assert.equal(res.status, 405);
});

test("/healthz is unchanged", async () => {
  const res = await call("/healthz", {});
  assert.equal(res.status, 200);
  const body = await res.json();
  assert.equal(body.ok, true);
  assert.equal(body.configured, false);
  assert.equal(body.version, null);
});
