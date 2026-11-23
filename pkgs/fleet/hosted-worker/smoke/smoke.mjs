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
import { FixtureSessionRegistry } from "../src/sessions.ts";
import { bearerReq, makeHarness } from "../test/helpers.ts";

const h = await makeHarness();
const deps = { ...h.deps, sessions: new FixtureSessionRegistry() };
const base = "https://fleet-hosted.test";
let failures = 0;

async function check(name, req, wantStatus, want) {
  const res = await dispatch(req, deps);
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

// Session registry (#12584): open -> resolve -> close -> resolve
// refused, through the real router.
const sessionPost = (path, body) =>
  bearerReq(path, token, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });

const opened = await check(
  "POST /v1/sessions (open)",
  sessionPost("/v1/sessions", {
    lane: "code-a",
    model: "model-x",
    lane_start_issues: ["developing-today/code#12584"],
  }),
  201,
  (b) => /^ses_[0-9a-f]{32}$/.test(b.session_id) && b.closed_at === null,
);
console.log(`     minted session: ${opened.session_id}`);

await check(
  "POST /v1/sessions/resolve (open session)",
  sessionPost("/v1/sessions/resolve", { session_id: opened.session_id }),
  200,
  (b) => b.session_id === opened.session_id && b.lane === "code-a",
);

await check(
  "POST /v1/sessions/close",
  sessionPost("/v1/sessions/close", { session_id: opened.session_id }),
  200,
  (b) => typeof b.closed_at === "string",
);

await check(
  "POST /v1/sessions/resolve (closed session refused)",
  sessionPost("/v1/sessions/resolve", { session_id: opened.session_id }),
  409,
  (b) => b.error === "session_closed",
);

if (failures > 0) {
  console.error(`smoke: ${failures} check(s) failed`);
  process.exit(1);
}
console.log("smoke: all checks passed");
