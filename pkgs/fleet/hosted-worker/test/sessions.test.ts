// Session registry tests (#12584). Oracles: the issue's acceptance
// (open → bare recovery of the same id; a closed or unknown id
// refused with a named error; the record read back showing account,
// lane, model and lane-start issues as opened) and design §4.2 (the
// service mints the id; the record is keyed by the caller's
// binding_id; unknown/closed ids are refused on the resolve path).

import assert from "node:assert/strict";
import { test } from "node:test";
import type { AuthDeps } from "../src/auth.ts";
import { configFromEnv } from "../src/config.ts";
import { dispatch } from "../src/router.ts";
import {
  InMemorySessionStateStore,
  SessionResolutionError,
  resolveAndBindSession,
} from "../src/session-resolution.ts";
import { FixtureSessionRegistry } from "../src/sessions.ts";
import { AUDIENCE, BINDING_A, BINDING_B, ISSUER, SERVICE_TOKEN, bearerReq, makeHarness } from "./helpers.ts";

const h = await makeHarness();
const registry = new FixtureSessionRegistry();
const deps: AuthDeps = { ...h.deps, sessions: registry };
const tokenA = await h.sign(h.rpPayload());

// A second tenancy (binding B) sharing the same registry and provider
// key, for the cross-tenant cases.
const depsB: AuthDeps = {
  config: configFromEnv({
    RP_ISSUER: ISSUER,
    RP_AUDIENCE: AUDIENCE,
    TENANCY_FIXTURE_JSON: JSON.stringify({
      subjects: [{ issuer: ISSUER, subject: "user-sub-2", binding_id: BINDING_B, active: true }],
      service_tokens: [],
    }),
  }),
  jwksFetcher: h.deps.jwksFetcher,
  sessions: registry,
};
const tokenB = await h.sign(h.rpPayload({ sub: "user-sub-2" }));

interface SessionBody {
  session_id: string;
  binding_id: string;
  lane: string | null;
  model: string | null;
  opened_at: string;
  closed_at: string | null;
  lane_start_issues: string[];
  error?: string;
}

function post(path: string, token: string, body: unknown, d: AuthDeps = deps): Promise<Response> {
  return dispatch(
    bearerReq(path, token, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    }),
    d,
  );
}

async function openOk(token: string, body: unknown, d: AuthDeps = deps): Promise<SessionBody> {
  const res = await post("/v1/sessions", token, body, d);
  assert.equal(res.status, 201);
  return (await res.json()) as SessionBody;
}

test("open mints a server-side id; the record reads back as opened", async () => {
  const opened = await openOk(tokenA, {
    lane: "code-a",
    model: "model-x",
    lane_start_issues: ["developing-today/code#12584"],
  });
  // The minted shape: the native #9075 format, ses_ + 32 hex chars.
  assert.match(opened.session_id, /^ses_[0-9a-f]{32}$/);
  assert.equal(opened.binding_id, BINDING_A);
  assert.equal(opened.lane, "code-a");
  assert.equal(opened.model, "model-x");
  assert.deepEqual(opened.lane_start_issues, ["developing-today/code#12584"]);
  assert.equal(opened.closed_at, null);
  assert.ok(!Number.isNaN(Date.parse(opened.opened_at)));

  const res = await post("/v1/sessions/resolve", tokenA, { session_id: opened.session_id });
  assert.equal(res.status, 200);
  const resolved = (await res.json()) as SessionBody;
  assert.deepEqual(resolved, opened);
});

test("the client never chooses its session id and never asserts the binding", async () => {
  const opened = await openOk(tokenA, {
    session_id: "ses_chosen1",
    binding_id: BINDING_B,
    lane: "code-a",
  });
  assert.notEqual(opened.session_id, "ses_chosen1");
  assert.match(opened.session_id, /^ses_[0-9a-f]{32}$/);
  assert.equal(opened.binding_id, BINDING_A);
});

test("two opens mint distinct ids, both resolvable", async () => {
  const one = await openOk(tokenA, {});
  const two = await openOk(tokenA, {});
  assert.notEqual(one.session_id, two.session_id);
  for (const id of [one.session_id, two.session_id]) {
    const res = await post("/v1/sessions/resolve", tokenA, { session_id: id });
    assert.equal(res.status, 200);
  }
});

test("open validates its body", async () => {
  const badIssues = await post("/v1/sessions", tokenA, { lane_start_issues: [42] });
  assert.equal(badIssues.status, 400);
  assert.equal(((await badIssues.json()) as SessionBody).error, "invalid_body");
  const badLane = await post("/v1/sessions", tokenA, { lane: 7 });
  assert.equal(badLane.status, 400);
  const notJson = await dispatch(
    bearerReq("/v1/sessions", tokenA, { method: "POST", body: "not json" }),
    deps,
  );
  assert.equal(notJson.status, 400);
  assert.equal(((await notJson.json()) as SessionBody).error, "invalid_body");
});

test("resolve refuses an unknown id with session_unknown", async () => {
  const res = await post("/v1/sessions/resolve", tokenA, {
    session_id: "ses_00000000000000000000000000000000",
  });
  assert.equal(res.status, 404);
  assert.equal(((await res.json()) as SessionBody).error, "session_unknown");
});

test("a malformed session id fails closed before the registry", async () => {
  const res = await post("/v1/sessions/resolve", tokenA, { session_id: "not-a-session" });
  assert.equal(res.status, 400);
  assert.equal(((await res.json()) as SessionBody).error, "invalid_session_id");
});

test("resume returns the record of an open session", async () => {
  const opened = await openOk(tokenA, { lane: "code-a" });
  const res = await post("/v1/sessions/resume", tokenA, { session_id: opened.session_id });
  assert.equal(res.status, 200);
  assert.equal(((await res.json()) as SessionBody).session_id, opened.session_id);
});

test("close ends the session: resolve, resume and a second close refuse it as session_closed", async () => {
  const opened = await openOk(tokenA, { lane: "code-a" });
  const closedRes = await post("/v1/sessions/close", tokenA, { session_id: opened.session_id });
  assert.equal(closedRes.status, 200);
  const closed = (await closedRes.json()) as SessionBody;
  assert.ok(closed.closed_at !== null && Date.parse(closed.closed_at) >= Date.parse(closed.opened_at));

  for (const path of ["/v1/sessions/resolve", "/v1/sessions/resume", "/v1/sessions/close"]) {
    const res = await post(path, tokenA, { session_id: opened.session_id });
    assert.equal(res.status, 409, path);
    assert.equal(((await res.json()) as SessionBody).error, "session_closed", path);
  }
});

test("another tenancy's session is unknown here and stays open for its owner", async () => {
  const opened = await openOk(tokenA, { lane: "code-a" });
  const resolveB = await post("/v1/sessions/resolve", tokenB, { session_id: opened.session_id }, depsB);
  assert.equal(resolveB.status, 404);
  assert.equal(((await resolveB.json()) as SessionBody).error, "session_unknown");
  const closeB = await post("/v1/sessions/close", tokenB, { session_id: opened.session_id }, depsB);
  assert.equal(closeB.status, 404);
  const resolveA = await post("/v1/sessions/resolve", tokenA, { session_id: opened.session_id });
  assert.equal(resolveA.status, 200);
});

test("an agent caller opens under its credential lane; a different declared lane is refused", async () => {
  const agentToken = await h.sign(
    h.rpPayload({ sub: "agent-sub-1", roles: ["agent"], lane: "code-a", session_id: "ses_cred" }),
  );
  const inherited = await openOk(agentToken, {});
  assert.equal(inherited.lane, "code-a");
  const matching = await openOk(agentToken, { lane: "code-a" });
  assert.equal(matching.lane, "code-a");
  const beyond = await post("/v1/sessions", agentToken, { lane: "code-b" });
  assert.equal(beyond.status, 403);
  assert.equal(((await beyond.json()) as SessionBody).error, "lane_beyond_credential");
});

test("a service token not scoped to the sessions door is refused", async () => {
  const res = await post("/v1/sessions", SERVICE_TOKEN, { lane: "code-a" });
  assert.equal(res.status, 403);
  assert.equal(((await res.json()) as SessionBody).error, "service_token_route_scope");
});

test("session routes refuse the unauthenticated and fail closed without a registry", async () => {
  const anon = await dispatch(
    new Request("https://fleet-hosted.test/v1/sessions", { method: "POST" }),
    deps,
  );
  assert.equal(anon.status, 401);
  const noRegistry = await post("/v1/sessions", tokenA, {}, h.deps);
  assert.equal(noRegistry.status, 503);
  assert.equal(((await noRegistry.json()) as SessionBody).error, "session_registry_not_configured");
});

test("acceptance: a later bare invocation recovers the opened session; after close it refuses stale", async () => {
  const opened = await openOk(tokenA, { lane: "code-a", model: "model-x" });
  // The CLI stored the id in its per-workspace state at open time.
  const store = new InMemorySessionStateStore();
  store.write(opened.session_id);

  // A later bare invocation: no explicit id, no env — only the stored
  // state and the registry's word on liveness (fetched here the way a
  // CLI resolve call would fetch it).
  const liveRecord = await registry.get(opened.session_id);
  const recovered = resolveAndBindSession({
    store,
    isLive: (id) => liveRecord !== null && liveRecord.sessionId === id && liveRecord.closedAt === null,
  });
  assert.deepEqual(recovered, { kind: "session", sessionId: opened.session_id, source: "stored" });

  await post("/v1/sessions/close", tokenA, { session_id: opened.session_id });
  const closedRecord = await registry.get(opened.session_id);
  assert.throws(
    () =>
      resolveAndBindSession({
        store,
        isLive: (id) =>
          closedRecord !== null && closedRecord.sessionId === id && closedRecord.closedAt === null,
      }),
    (e: unknown) => e instanceof SessionResolutionError && e.code === "stale_session_state",
  );
});
