// Hermetic tests for the rehost Worker's GET /version route.
// Oracle: #15793 criterion 1 — each in-service Worker answers
// `GET /version` unauthenticated with the commit it was deployed
// from — and the edge worker-checks probe
// (pm-stack/console/edge/src/worker-checks.ts, commitOf), which
// reads the `version` field of that JSON answer.

import assert from "node:assert/strict";
import test from "node:test";

import worker from "../src/index.js";

const SHA = "a".repeat(40);

test("GET /version answers the stamped deployed commit unauthenticated", async () => {
  const res = await worker.fetch(new Request("https://git.security.cab/version"), {
    VERSION: SHA,
  });
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), {
    ok: true,
    service: "fleet-git-rehost",
    version: SHA,
  });
});

test("GET /version names no commit when the deploy did not stamp one", async () => {
  const res = await worker.fetch(new Request("https://git.security.cab/version"), {});
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), {
    ok: true,
    service: "fleet-git-rehost",
    version: "unknown",
  });
});

test("POST /version is refused", async () => {
  const res = await worker.fetch(
    new Request("https://git.security.cab/version", { method: "POST" }),
    { VERSION: SHA },
  );
  assert.equal(res.status, 405);
});

test("the root page still answers without any R2 access", async () => {
  const res = await worker.fetch(new Request("https://git.security.cab/"), {});
  assert.equal(res.status, 200);
  assert.match(await res.text(), /fleet-git-rehost/);
});
