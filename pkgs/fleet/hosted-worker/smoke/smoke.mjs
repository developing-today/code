// Local-run smoke for fleet-hosted (#12583): boots the real router
// in-process with a fixture RP provider key + tenancy fixture (the
// test harness), then answers the acceptance questions against it —
// health and capabilities answer with the hosted door's identity, a
// door refuses the unauthenticated, and an authenticated caller
// reaches the door stub. For a wrangler dev run instead: set RP_ISSUER
// / TENANCY_FIXTURE_JSON in the environment and `npm run dev`, then
// curl /health and /capabilities.
//
// Run: npm run smoke

import { dispatch } from "../src/router.ts";
import { bearerReq, makeHarness } from "../test/helpers.ts";

const h = await makeHarness();
const base = "https://fleet-hosted.test";
let failures = 0;

async function check(name, req, wantStatus, want) {
  const res = await dispatch(req, h.deps);
  const body = await res.json();
  const okStatus = res.status === wantStatus;
  const okBody = want ? want(body) : true;
  console.log(`${okStatus && okBody ? "ok  " : "FAIL"} ${name}: HTTP ${res.status}`);
  if (!okStatus || !okBody) {
    failures++;
    console.log(`     body: ${JSON.stringify(body)}`);
  }
  return body;
}

const health = await check("GET /health", new Request(`${base}/health`), 200, (b) => b.service === "fleet-hosted" && b.status === "ok");
console.log(`     identity: ${health.service} v${health.version}; doors: ${health.doors.join(", ")}`);

await check(
  "GET /capabilities",
  new Request(`${base}/capabilities`),
  200,
  (b) => b.build?.name === "fleet-hosted" && Array.isArray(b.domains) && b.domains.length === 12,
);

await check(
  "POST /v1/stamps (no token)",
  new Request(`${base}/v1/stamps`, { method: "POST" }),
  401,
  (b) => b.error === "missing_token",
);

const token = await h.sign(h.rpPayload());
await check(
  "POST /v1/stamps (valid RP token)",
  bearerReq("/v1/stamps", token, { method: "POST" }),
  501,
  (b) => b.error === "not_implemented",
);

if (failures > 0) {
  console.error(`smoke: ${failures} check(s) failed`);
  process.exit(1);
}
console.log("smoke: all checks passed");
