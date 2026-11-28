// Behavioral tests for the #15793 /version answer — oracle: #15793
// criterion 1 plus the edge probe contract (`commitOf` in
// pm-stack/console/edge/src/worker-checks.ts reads `version`, then
// `commit`, from the /version JSON answer). Plain Node, no Worker
// bindings: versionAnswer is a pure helper in src/lib.ts, and
// src/index.ts answers GET /version with it (the wiring is pinned by
// test/test_version_route.py).
import assert from "node:assert/strict";
import { test } from "node:test";
import { versionAnswer } from "../src/lib.ts";

test("versionAnswer names the stamped commit (oracle: probe commitOf reads the version field)", () => {
  assert.deepEqual(versionAnswer("3b40a022e0e81344f94447668edc71f6cf2d477d"), {
    ok: true,
    service: "lab-ops-hub",
    version: "3b40a022e0e81344f94447668edc71f6cf2d477d",
  });
});

test("versionAnswer answers unknown when unstamped (oracle: the probe records it honestly)", () => {
  assert.equal(versionAnswer(undefined).version, "unknown");
  assert.equal(versionAnswer("").version, "unknown");
});

test("versionAnswer carries the pm-service/github-ci answer shape (oracle: {ok, service, version})", () => {
  const answer = versionAnswer("abc123");
  assert.equal(answer.ok, true);
  assert.equal(answer.service, "lab-ops-hub");
  assert.equal(answer.version, "abc123");
});
