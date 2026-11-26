import assert from "node:assert/strict";
import { assertTargetIdentity } from "./check-target.mjs";
import { resolveDeployableVersion, selectTaggedDeployableVersion } from "./resolve-deployable-version.mjs";

const payload = {
  success: true,
  result: [
    { id: "older", created_on: "2026-01-01T00:00:00Z", annotations: { "workers/tag": "git-0123456789ab" } },
    { id: "other", created_on: "2026-02-01T00:00:00Z", annotations: { "workers/tag": "git-aaaaaaaaaaaa" } },
    { id: "newer", created_on: "2026-03-01T00:00:00Z", annotations: { "workers/tag": "git-0123456789ab" } },
  ],
};

assert.equal(selectTaggedDeployableVersion(payload, "git-0123456789ab"), "newer");
assert.equal(selectTaggedDeployableVersion({ success: true, result: { items: payload.result } }, "git-0123456789ab"), "newer");
assert.throws(() => selectTaggedDeployableVersion({ success: false, result: [] }, "git-0123456789ab"));
assert.throws(() => selectTaggedDeployableVersion({ success: true, result: [] }, "git-0123456789ab"));

let request;
const id = await resolveDeployableVersion({
  tag: "git-0123456789ab",
  accountId: "0123456789abcdef0123456789abcdef",
  apiToken: "test-only-token",
  fetchImpl: async (url, options) => {
    request = { url: new URL(url), options };
    return { ok: true, status: 200, json: async () => payload };
  },
});
assert.equal(id, "newer");
assert.equal(request.url.pathname, "/client/v4/accounts/0123456789abcdef0123456789abcdef/workers/scripts/decision-events/versions");
assert.equal(request.url.searchParams.get("deployable"), "true");
assert.equal(request.url.searchParams.get("per_page"), "100");
assert.equal(request.options.headers.Authorization, "Bearer test-only-token");

assert.doesNotThrow(() =>
  assertTargetIdentity(
    { result: { name: "decision-events" } },
    { result: { name: "decision-events", uuid: "68ade85e-6835-46a8-af6a-a9f2f2d966ae" } },
  ),
);
assert.throws(() =>
  assertTargetIdentity(
    { result: { name: "another-worker" } },
    { result: { name: "decision-events", uuid: "68ade85e-6835-46a8-af6a-a9f2f2d966ae" } },
  ),
);
assert.throws(() =>
  assertTargetIdentity(
    { result: { name: "decision-events" } },
    { result: { name: "decision-events", uuid: "00000000-0000-0000-0000-000000000000" } },
  ),
);

console.log("Release target and version selection tests passed without Cloudflare credentials or network access.");
