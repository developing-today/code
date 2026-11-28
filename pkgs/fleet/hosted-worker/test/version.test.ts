// /version tests (#15793 criterion 1). Oracle: the edge probe's
// contract (`commitOf` in pm-stack/console/edge/src/worker-checks.ts
// tries GET /version first and reads the `version` field) — the
// route must answer unauthenticated with the commit stamped as the
// VERSION var at deploy time, and "unknown" when unstamped, never
// the package version /health reports.

import assert from "node:assert/strict";
import { test } from "node:test";
import { configFromEnv, HOSTED_VERSION } from "../src/config.ts";
import { versionAnswer } from "../src/health.ts";
import { dispatch } from "../src/router.ts";
import { buildDeps } from "../src/worker.ts";

const SHA = "0123456789abcdef0123456789abcdef01234567";

test("versionAnswer names the stamped deployed commit", () => {
  assert.deepEqual(versionAnswer(configFromEnv({ VERSION: SHA })), {
    ok: true,
    service: "fleet-hosted",
    version: SHA,
  });
});

test("versionAnswer is honest when unstamped", () => {
  assert.deepEqual(versionAnswer(configFromEnv({})), {
    ok: true,
    service: "fleet-hosted",
    version: "unknown",
  });
  assert.equal(versionAnswer(configFromEnv({ VERSION: "   " })).version, "unknown");
});

test("GET /version answers publicly with the stamped commit", async () => {
  const res = await dispatch(new Request("https://fleet-hosted.test/version"), buildDeps({ VERSION: SHA }));
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), { ok: true, service: "fleet-hosted", version: SHA });
});

test("GET /version answers unknown when no VERSION var is stamped", async () => {
  const res = await dispatch(new Request("https://fleet-hosted.test/version"), buildDeps({}));
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), { ok: true, service: "fleet-hosted", version: "unknown" });
});

test("non-GET /version is the Worker's plain 404", async () => {
  const res = await dispatch(
    new Request("https://fleet-hosted.test/version", { method: "POST" }),
    buildDeps({ VERSION: SHA }),
  );
  assert.equal(res.status, 404);
});

test("/health is unchanged: its version stays the package version", async () => {
  const res = await dispatch(new Request("https://fleet-hosted.test/health"), buildDeps({ VERSION: SHA }));
  assert.equal(res.status, 200);
  const body = (await res.json()) as { status: string; service: string; version: string };
  assert.equal(body.status, "ok");
  assert.equal(body.service, "fleet-hosted");
  assert.equal(body.version, HOSTED_VERSION);
});
