// Local-run smoke for fleet-hosted (#12583): boots the real router
// in-process with a fixture RP provider key + tenancy fixture (the
// test harness), then answers the acceptance questions against it —
// health and capabilities answer with the hosted door's identity, a
// door refuses the unauthenticated, the session registry opens /
// resolves / closes (#12584), and the stamping door creates a
// stamped, signed item (#12585). For a wrangler dev run instead: set
// RP_ISSUER / TENANCY_FIXTURE_JSON in the environment and
// `npm run dev`, then curl /health and /capabilities.
//
// Run: npm run smoke

import { configFromEnv } from "../src/config.ts";
import { FixtureGitHubWriter } from "../src/github.ts";
import { dispatch } from "../src/router.ts";
import { FixtureSessionRegistry } from "../src/sessions.ts";
import { FixtureMetadataStore } from "../src/stamping.ts";
import { AUDIENCE, BINDING_A, ISSUER, bearerReq, makeHarness } from "../test/helpers.ts";

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

// Stamping door (#12585): a create through the real router with a
// fixture stamp key + fixture store/writer. The caller's block
// carries a forged picked_up_at; the stamp must overwrite it from
// service sources while preserving the caller-owned direct_parent.
const b64url = (bytes) => {
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
};
const stampKeyPair = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
const stampDeps = {
  config: configFromEnv({
    RP_ISSUER: ISSUER,
    RP_AUDIENCE: AUDIENCE,
    TENANCY_FIXTURE_JSON: JSON.stringify({
      subjects: [{ issuer: ISSUER, subject: "user-sub-1", binding_id: BINDING_A, active: true }],
      service_tokens: [],
    }),
    STAMP_SIGNING_KEY_PKCS8: b64url(new Uint8Array(await crypto.subtle.exportKey("pkcs8", stampKeyPair.privateKey))),
    STAMP_SIGNING_KEY_ID: "smoke-key",
    STAMP_VERIFY_KEYS_JSON: JSON.stringify({
      "smoke-key": b64url(new Uint8Array(await crypto.subtle.exportKey("raw", stampKeyPair.publicKey))),
    }),
  }),
  jwksFetcher: h.deps.jwksFetcher,
  sessions: new FixtureSessionRegistry(),
  metadataStore: new FixtureMetadataStore(),
  githubWriter: new FixtureGitHubWriter(),
};
const callerBody = [
  "Smoke item",
  "",
  "<!-- pm-metadata:start -->",
  "```json",
  JSON.stringify({
    pm_meta: 1,
    direct_parent: { repo: "developing-today/code", number: 9081, kind: "design" },
    provenance: { picked_up_at: "1970-01-01T00:00:00.000Z" },
  }),
  "```",
  "<!-- pm-metadata:end -->",
].join("\n");
const stamped = await dispatch(
  bearerReq("/v1/stamps", token, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ repo: "developing-today/code", kind: "issue", title: "Smoke create", body: callerBody }),
  }),
  stampDeps,
);
{
  const body = await stamped.json();
  const ok =
    stamped.status === 201 &&
    body.metadata_inserted === true &&
    body.metadata.direct_parent?.number === 9081 &&
    body.metadata.provenance.picked_up_at === null &&
    String(body.metadata.provenance.stamp_sig).startsWith("v1.ed25519.smoke-key.");
  console.log(`${ok ? "ok  " : "FAIL"} POST /v1/stamps (stamped create): HTTP ${stamped.status}`);
  if (!ok) {
    failures++;
    console.log(`     body: ${JSON.stringify(body)}`);
  }
}

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
