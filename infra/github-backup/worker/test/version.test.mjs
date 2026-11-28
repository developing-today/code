import assert from "node:assert/strict";
import { test } from "node:test";
import worker, { versionAnswer } from "../src/index.js";

// Oracle: #15793 criterion 1 — each in-service Worker answers
// unauthenticated GET /version with its deployed commit, in the
// `version`-field shape the edge probe reads (commitOf in
// pm-stack/console/edge/src/worker-checks.ts).

test("GET /version names the stamped deployed commit, unauthenticated", async () => {
  const env = { VERSION: "abc123def456" };
  const res = await worker.fetch(new Request("https://worker.test/version"), env);
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), {
    ok: true,
    service: "developing-today-backup",
    version: "abc123def456",
  });
});

test("GET /version answers unknown when unstamped or blank", async () => {
  for (const env of [{}, { VERSION: "" }, { VERSION: "   " }]) {
    const res = await worker.fetch(new Request("https://worker.test/version"), env);
    assert.equal(res.status, 200);
    assert.deepEqual(await res.json(), {
      ok: true,
      service: "developing-today-backup",
      version: "unknown",
    });
  }
  assert.deepEqual(versionAnswer({ VERSION: "  abc123  " }), {
    ok: true,
    service: "developing-today-backup",
    version: "abc123",
  });
});

test("non-GET /version is a 405", async () => {
  const res = await worker.fetch(
    new Request("https://worker.test/version", { method: "POST" }),
    { VERSION: "abc123" },
  );
  assert.equal(res.status, 405);
});

test("/health and the /queue bearer gate are unchanged", async () => {
  const health = await worker.fetch(new Request("https://worker.test/health"), {});
  assert.equal(health.status, 200);
  assert.equal(await health.text(), "ok");

  const queue = await worker.fetch(
    new Request("https://worker.test/queue"),
    { GHB_EVENTS: {}, DRAIN_BEARER: "test-bearer" },
  );
  assert.equal(queue.status, 401);
});
