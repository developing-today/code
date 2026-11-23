// Local-run smoke for fleet-hosted (#12583): boots the real router
// in-process with a fixture RP provider key + tenancy fixture (the
// test harness), then answers the acceptance questions against it —
// health and capabilities answer with the hosted door's identity, a
// door refuses the unauthenticated, the session registry opens /
// resolves / closes (#12584), the stamping door creates a
// stamped, signed item (#12585), the query door finds it by its
// Metadata fields (#12587), and the queue door runs a full broker
// lifecycle — enqueue, claim, heartbeat, checkpoint, park, wake,
// claim, complete — plus a fallback claim over the comment
// surface (#12589), and the ingestion door stores the created
// item into a fresh metadata store from a signed webhook
// delivery, then re-checks it by sweep (#12590), and the sync
// door runs project-reconcile's write set end to end and posts
// a verdict flow note exactly once (#12591). For a wrangler
// dev run instead: set
// RP_ISSUER / TENANCY_FIXTURE_JSON in the environment and
// `npm run dev`, then curl /health and /capabilities.
//
// Run: npm run smoke

import { sha256Hex } from "../src/auth.ts";
import { configFromEnv } from "../src/config.ts";
import { FixtureGitHubWriter } from "../src/github.ts";
import { FixtureQueueStore } from "../src/queue-store.ts";
import { StoreQueueRecordSource } from "../src/queue-records.ts";
import { dispatch } from "../src/router.ts";
import { FixtureSessionRegistry } from "../src/sessions.ts";
import { FixtureMetadataStore } from "../src/stamping.ts";
import { AUDIENCE, BINDING_A, ISSUER, WEBHOOK_SECRET, bearerReq, makeHarness } from "../test/helpers.ts";

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
const smokeWriter = new FixtureGitHubWriter();
const stampEnv = {
  STAMP_SIGNING_KEY_PKCS8: b64url(new Uint8Array(await crypto.subtle.exportKey("pkcs8", stampKeyPair.privateKey))),
  STAMP_SIGNING_KEY_ID: "smoke-key",
  STAMP_VERIFY_KEYS_JSON: JSON.stringify({
    "smoke-key": b64url(new Uint8Array(await crypto.subtle.exportKey("raw", stampKeyPair.publicKey))),
  }),
};
const stampDeps = {
  config: configFromEnv({
    RP_ISSUER: ISSUER,
    RP_AUDIENCE: AUDIENCE,
    TENANCY_FIXTURE_JSON: JSON.stringify({
      subjects: [{ issuer: ISSUER, subject: "user-sub-1", binding_id: BINDING_A, active: true }],
      service_tokens: [],
    }),
    ...stampEnv,
  }),
  jwksFetcher: h.deps.jwksFetcher,
  sessions: new FixtureSessionRegistry(),
  metadataStore: new FixtureMetadataStore(),
  githubWriter: smokeWriter,
  githubItems: smokeWriter,
  githubComments: smokeWriter,
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

// Query door (#12587): the just-stamped item is findable by its
// Metadata fields, served from the DB with its body checked and no
// divergence; the same filter over POST answers identically.
{
  const viaGet = await dispatch(
    bearerReq(`/v1/query?repo=${encodeURIComponent("developing-today/code")}`, token),
    stampDeps,
  );
  const getDoc = await viaGet.json();
  const found = getDoc.results?.find((r) => r.number === 1);
  const okGet =
    viaGet.status === 200 &&
    found?.source === "db" &&
    found?.body_checked === true &&
    found?.divergence === null &&
    found?.stamp?.status === "verified";
  console.log(`${okGet ? "ok  " : "FAIL"} GET /v1/query (stamped item, no divergence): HTTP ${viaGet.status}`);
  if (!okGet) {
    failures++;
    console.log(`     body: ${JSON.stringify(getDoc)}`);
  }

  const viaPost = await dispatch(
    bearerReq("/v1/query", token, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ direct_parent: "developing-today/code#9081" }),
    }),
    stampDeps,
  );
  const postDoc = await viaPost.json();
  const okPost = viaPost.status === 200 && postDoc.count === 1 && postDoc.results[0].number === 1;
  console.log(`${okPost ? "ok  " : "FAIL"} POST /v1/query (by direct_parent): HTTP ${viaPost.status}`);
  if (!okPost) {
    failures++;
    console.log(`     body: ${JSON.stringify(postDoc)}`);
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

// Queue door (#12589): the full broker lifecycle against a fixture
// queue store wired like worker.ts wires it (with the record
// source the stamping door reads), each verb answering its broker
// envelope at HTTP 200.
{
  const queueStore = new FixtureQueueStore();
  const queueSessions = new FixtureSessionRegistry();
  const queueDeps = {
    ...stampDeps,
    sessions: queueSessions,
    queueStore,
    queueRecords: new StoreQueueRecordSource(queueStore, queueSessions),
  };
  const queuePost = (path, body) =>
    bearerReq(path, token, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
  const run = async (name, path, body, want) => {
    const res = await dispatch(queuePost(path, body), queueDeps);
    const doc = await res.json();
    const ok = res.status === 200 && doc.ok === true && (want ? want(doc) : true);
    console.log(`${ok ? "ok  " : "FAIL"} ${name}: HTTP ${res.status}`);
    if (!ok) {
      failures++;
      console.log(`     body: ${JSON.stringify(doc)}`);
    }
    return doc;
  };
  const packet = {
    title: "smoke fix",
    priority: 30,
    dedupe_key: "repo-health:v1:fix:developing-today/code:issue:12589",
    payload: {
      schema: "repo-task/v1",
      kind: "small_fix",
      repo: "developing-today/code",
      issue: { number: 12589 },
      problem: "smoke problem",
      priority_reason: "smoke",
      evidence: ["https://example.test/evidence"],
      acceptance: ["smoke passes"],
      tests: ["npm run smoke"],
      context_refs: [],
      scope: { include: ["pkgs/fleet/hosted-worker"], exclude: ["docs"] },
      risk: { level: "low", flags: [] },
      timebox_minutes: 45,
      dedupe: { checked_at: new Date().toISOString(), basis: "smoke" },
    },
  };
  const enq = await run("POST /v1/queue/enqueue", "/v1/queue/enqueue", { queue: "repo-health", packet }, (d) => d.row?.status === "queued");
  const taskId = enq.row.id;
  const claimed = await run("POST /v1/queue/claim", "/v1/queue/claim", { worker: "smoke-worker" }, (d) => d.row?.worker_id === "smoke-worker");
  await run("POST /v1/queue/heartbeat", "/v1/queue/heartbeat", { task_id: taskId, worker: "smoke-worker" }, (d) => d.row?.status === "claimed");
  await run("POST /v1/queue/checkpoint", "/v1/queue/checkpoint", { task_id: taskId, worker: "smoke-worker", stage: "working" }, (d) => d.row?.progress?.last_checkpoint?.stage === "working");
  await run("POST /v1/queue/park", "/v1/queue/park", { task_id: taskId, worker: "smoke-worker", delay_seconds: 60 }, (d) => d.row?.status === "parked");
  await run("POST /v1/queue/wake", "/v1/queue/wake", { task_id: taskId, worker: "smoke-worker" }, (d) => d.row?.status === "queued");
  await run("POST /v1/queue/claim (after wake)", "/v1/queue/claim", { worker: "smoke-worker" }, (d) => d.row?.id === taskId);
  const nowIsoReal = new Date().toISOString();
  await run(
    "POST /v1/queue/complete",
    "/v1/queue/complete",
    {
      task_id: taskId,
      worker: "smoke-worker",
      result: {
        evidence: "commit abc123",
        disposition: {
          checked_at: nowIsoReal,
          source: "smoke",
          issue: { number: 12589, state: "closed" },
          pulls: [],
          verdict: "closed_verified",
        },
      },
    },
    (d) => d.row?.status === "done",
  );
  {
    // The fallback verbs answer the native CLI's result JSON, not
    // the broker envelope — claimed:true is the success shape.
    const res = await dispatch(
      queuePost("/v1/queue/fallback/claim", { repo: "developing-today/code", number: 12589, worker: "smoke-worker" }),
      queueDeps,
    );
    const doc = await res.json();
    const ok = res.status === 200 && doc.claimed === true;
    console.log(`${ok ? "ok  " : "FAIL"} POST /v1/queue/fallback/claim: HTTP ${res.status}`);
    if (!ok) {
      failures++;
      console.log(`     body: ${JSON.stringify(doc)}`);
    }
  }
  void claimed;
}

// Ingestion door (#12590): the item the stamping door just
// created exists on GitHub (the shared fixture writer) but NOT
// in this block's fresh metadata store — the store that missed
// it. A signed webhook delivery stores it (the heal); a delivery
// with a bad signature is refused 401; the sweeper then finds
// the row already matching.
{
  const ingestToken = "smoke-ingestion-token-0123456789ab";
  const ingestDeps = {
    config: configFromEnv({
      RP_ISSUER: ISSUER,
      RP_AUDIENCE: AUDIENCE,
      TENANCY_FIXTURE_JSON: JSON.stringify({
        subjects: [{ issuer: ISSUER, subject: "user-sub-1", binding_id: BINDING_A, active: true }],
        service_tokens: [
          {
            sha256: await sha256Hex(ingestToken),
            binding_id: BINDING_A,
            routes: ["ingestion"],
            active: true,
            label: "smoke-ingestion",
          },
        ],
      }),
      GITHUB_WEBHOOK_SECRET: WEBHOOK_SECRET,
      ...stampEnv,
    }),
    jwksFetcher: h.deps.jwksFetcher,
    sessions: new FixtureSessionRegistry(),
    metadataStore: new FixtureMetadataStore(),
    githubWriter: smokeWriter,
    githubItems: smokeWriter,
    githubEditor: smokeWriter,
    githubComments: smokeWriter,
    queueStore: new FixtureQueueStore(),
  };
  const signSmoke = async (raw) => {
    const key = await crypto.subtle.importKey(
      "raw",
      new TextEncoder().encode(WEBHOOK_SECRET),
      { name: "HMAC", hash: "SHA-256" },
      false,
      ["sign"],
    );
    const sig = new Uint8Array(await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(raw)));
    return `sha256=${[...sig].map((b) => b.toString(16).padStart(2, "0")).join("")}`;
  };
  const payload = JSON.stringify({
    action: "opened",
    repository: { full_name: "developing-today/code" },
    issue: { number: 1 },
  });
  const hookReq = (signature) =>
    new Request(`${base}/v1/ingestion/github`, {
      method: "POST",
      headers: {
        "x-github-event": "issues",
        "x-github-delivery": "smoke-delivery-1",
        "x-hub-signature-256": signature,
      },
      body: payload,
    });

  const hookRes = await dispatch(hookReq(await signSmoke(payload)), ingestDeps);
  const hookDoc = await hookRes.json();
  const okHook =
    hookRes.status === 200 && hookDoc.outcome === "inserted" && hookDoc.binding_id === BINDING_A;
  console.log(`${okHook ? "ok  " : "FAIL"} POST /v1/ingestion/github (signed delivery heals the store): HTTP ${hookRes.status}`);
  if (!okHook) {
    failures++;
    console.log(`     body: ${JSON.stringify(hookDoc)}`);
  }

  const badRes = await dispatch(hookReq(await signSmoke(`${payload} `)), ingestDeps);
  const badDoc = await badRes.json();
  const okBad = badRes.status === 401 && badDoc.error === "webhook_signature_invalid";
  console.log(`${okBad ? "ok  " : "FAIL"} POST /v1/ingestion/github (bad signature refused): HTTP ${badRes.status}`);
  if (!okBad) {
    failures++;
    console.log(`     body: ${JSON.stringify(badDoc)}`);
  }

  const sweepRes = await dispatch(
    bearerReq("/v1/ingestion/sweep", ingestToken, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ repos: ["developing-today/code"] }),
    }),
    ingestDeps,
  );
  const sweepDoc = await sweepRes.json();
  const okSweep =
    sweepRes.status === 200 &&
    sweepDoc.counts?.checked === 1 &&
    sweepDoc.results?.[0]?.outcome === "unchanged";
  console.log(`${okSweep ? "ok  " : "FAIL"} POST /v1/ingestion/sweep (row already matching): HTTP ${sweepRes.status}`);
  if (!okSweep) {
    failures++;
    console.log(`     body: ${JSON.stringify(sweepDoc)}`);
  }
}

// Sync door (#12591): project-reconcile's write set (an
// item-add with its initial status + a Status set) runs end to
// end over a fixture project; a full retry writes nothing
// twice; a verdict flow posts its marked note exactly once.
{
  const syncWriter = new FixtureGitHubWriter();
  syncWriter.seedItem({ repo: "developing-today/code", kind: "issue", number: 7, body: "tracked" });
  syncWriter.seedItem({ repo: "developing-today/code", kind: "issue", number: 42, body: "discovered" });
  syncWriter.seedProject({
    owner: "developing-today",
    number: 3,
    items: [{ repo: "developing-today/code", kind: "issue", number: 7, values: { Status: "Todo" } }],
  });
  const syncDeps = {
    ...h.deps,
    sessions: new FixtureSessionRegistry(),
    githubProjects: syncWriter,
    githubComments: syncWriter,
  };
  const syncToken = await h.sign(h.rpPayload());
  const syncPost = (path, body) =>
    dispatch(
      bearerReq(path, syncToken, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      }),
      syncDeps,
    );
  const ops = {
    project: { owner: "developing-today", number: 3 },
    ops: [
      { op: "add-issue", repo: "developing-today/code", number: 42, status: "Todo" },
      { op: "set-status", issue: "developing-today/code#7", to: "Done" },
    ],
  };
  const syncRes = await syncPost("/v1/sync/project", ops);
  const syncDoc = await syncRes.json();
  const okSync =
    syncRes.status === 200 &&
    syncDoc.results?.[0]?.outcome === "added" &&
    syncDoc.results?.[1]?.outcome === "status_set" &&
    syncDoc.counts?.applied === 2;
  console.log(`${okSync ? "ok  " : "FAIL"} POST /v1/sync/project (reconcile write set end to end): HTTP ${syncRes.status}`);
  if (!okSync) {
    failures++;
    console.log(`     body: ${JSON.stringify(syncDoc)}`);
  }

  const retryRes = await syncPost("/v1/sync/project", ops);
  const retryDoc = await retryRes.json();
  const okRetry =
    retryRes.status === 200 &&
    retryDoc.results?.every((r) => r.outcome.startsWith("already")) &&
    retryDoc.counts?.already === 2;
  console.log(`${okRetry ? "ok  " : "FAIL"} POST /v1/sync/project (full retry writes nothing twice): HTTP ${retryRes.status}`);
  if (!okRetry) {
    failures++;
    console.log(`     body: ${JSON.stringify(retryDoc)}`);
  }

  const flow = {
    flow: "verdict",
    repo: "developing-today/code",
    number: 7,
    marker: "smoke-cycle-1",
    body: "Stewardship verdict: accept.",
  };
  const flowRes = await syncPost("/v1/sync/queue-github", flow);
  const flowDoc = await flowRes.json();
  const flowRetryRes = await syncPost("/v1/sync/queue-github", flow);
  const flowRetryDoc = await flowRetryRes.json();
  const okFlow =
    flowRes.status === 200 &&
    flowDoc.comment?.outcome === "posted" &&
    flowRetryDoc.comment?.outcome === "already_posted" &&
    syncWriter.comments.length === 1;
  console.log(`${okFlow ? "ok  " : "FAIL"} POST /v1/sync/queue-github (verdict posts exactly once): HTTP ${flowRes.status}`);
  if (!okFlow) {
    failures++;
    console.log(`     body: ${JSON.stringify(flowDoc)} / ${JSON.stringify(flowRetryDoc)}`);
  }
}

if (failures > 0) {
  console.error(`smoke: ${failures} check(s) failed`);
  process.exit(1);
}
console.log("smoke: all checks passed");
