// Auth plumbing tests (#12583). Oracle: design §4.1 (who may call,
// and how each caller kind resolves) and §3.3 rule 3 (a per-user Turso
// DB credential is not service authentication). Every refusal asserts
// its stable error code so the fail-closed path is named, not implied.

import assert from "node:assert/strict";
import { test } from "node:test";
import { authenticate } from "../src/auth.ts";
import { configFromEnv } from "../src/config.ts";
import { FailClosed } from "../src/errors.ts";
import { dispatch } from "../src/router.ts";
import { BINDING_A, SERVICE_TOKEN, bearerReq, makeHarness } from "./helpers.ts";

const h = await makeHarness();

async function refusal(promise: Promise<unknown>): Promise<FailClosed> {
  try {
    await promise;
  } catch (e) {
    assert.ok(e instanceof FailClosed, `expected FailClosed, got ${String(e)}`);
    return e;
  }
  assert.fail("expected the call to be refused");
}

test("a valid RP token resolves to the tenancy binding_id", async () => {
  const token = await h.sign(h.rpPayload());
  const ctx = await authenticate(bearerReq("/v1/stamps", token), h.deps);
  assert.equal(ctx.kind, "rp");
  assert.equal(ctx.bindingId, BINDING_A);
  assert.deepEqual(ctx.roles, ["owner"]);
});

test("a valid RP token passes auth on a door route (a stub door then answers 501)", async () => {
  const token = await h.sign(h.rpPayload());
  // The probe rides a still-stubbed door: it used the queue door
  // until #12589 landed it; the ingestion door (#12590) is the
  // remaining stub with the same shape.
  const res = await dispatch(bearerReq("/v1/ingestion/github", token, { method: "POST" }), h.deps);
  assert.equal(res.status, 501);
  const body = (await res.json()) as { error: string };
  assert.equal(body.error, "not_implemented");
});

test("an invalid signature is refused", async () => {
  const token = await h.signWithWrongKey(h.rpPayload());
  const e = await refusal(authenticate(bearerReq("/v1/stamps", token), h.deps));
  assert.equal(e.status, 401);
  assert.equal(e.code, "invalid_signature");
});

test("an expired token is refused", async () => {
  const now = Math.floor(Date.now() / 1000);
  const token = await h.sign(h.rpPayload({ exp: now - 60 }));
  const e = await refusal(authenticate(bearerReq("/v1/stamps", token), h.deps));
  assert.equal(e.status, 401);
  assert.equal(e.code, "token_expired");
});

test("a token for another audience is refused", async () => {
  const token = await h.sign(h.rpPayload({ aud: "some-other-service" }));
  const e = await refusal(authenticate(bearerReq("/v1/stamps", token), h.deps));
  assert.equal(e.status, 401);
  assert.equal(e.code, "invalid_audience");
});

test("an absent token is refused on a door route", async () => {
  const res = await dispatch(new Request("https://fleet-hosted.test/v1/stamps", { method: "POST" }), h.deps);
  assert.equal(res.status, 401);
  const body = (await res.json()) as { error: string };
  assert.equal(body.error, "missing_token");
});

test("a token whose subject maps to no tenancy account fails closed", async () => {
  const token = await h.sign(h.rpPayload({ sub: "nobody-1" }));
  const e = await refusal(authenticate(bearerReq("/v1/stamps", token), h.deps));
  assert.equal(e.status, 403);
  assert.equal(e.code, "unknown_subject");
});

test("a token for an inactive tenancy account is refused", async () => {
  const token = await h.sign(h.rpPayload({ sub: "dormant-sub" }));
  const e = await refusal(authenticate(bearerReq("/v1/stamps", token), h.deps));
  assert.equal(e.status, 403);
  assert.equal(e.code, "account_inactive");
});

test("an agent credential resolves with its lane and session as the ceiling", async () => {
  const token = await h.sign(
    h.rpPayload({ sub: "agent-sub-1", roles: ["agent"], lane: "code-a", session_id: "sess-123" }),
  );
  const ctx = await authenticate(bearerReq("/v1/queue", token), h.deps);
  assert.equal(ctx.kind, "agent");
  assert.equal(ctx.bindingId, BINDING_A);
  assert.equal(ctx.lane, "code-a");
  assert.equal(ctx.sessionId, "sess-123");
});

test("an agent credential without lane/session claims is refused", async () => {
  const token = await h.sign(h.rpPayload({ sub: "agent-sub-1", roles: ["agent"] }));
  const e = await refusal(authenticate(bearerReq("/v1/queue", token), h.deps));
  assert.equal(e.status, 401);
  assert.equal(e.code, "agent_claims_incomplete");
});

test("a service token resolves on its scoped doors and is refused outside them", async () => {
  // The fixture token is scoped to stamping + query. The stamping
  // door is live (#12585), so an in-scope call gets past the scope
  // check into the handler — which refuses 503 here (the harness
  // config carries no stamp material) — while the unscoped queue
  // door refuses at the check itself. (The probe used the query
  // door's 501 stub until #12587 landed it; since #12589 the queue
  // door is a route per broker verb, so the probe names a real
  // queue route — the scope check fires before any handler runs.)
  const inScope = await dispatch(
    bearerReq("/v1/stamps", SERVICE_TOKEN, { method: "POST" }),
    h.deps,
  );
  assert.equal(inScope.status, 503);
  assert.equal(((await inScope.json()) as { error: string }).error, "stamp_signing_not_configured");

  const outOfScope = await dispatch(
    bearerReq("/v1/queue/health", SERVICE_TOKEN, { method: "POST" }),
    h.deps,
  );
  assert.equal(outOfScope.status, 403);
  const body = (await outOfScope.json()) as { error: string };
  assert.equal(body.error, "service_token_route_scope");
});

test("an unknown opaque token is refused", async () => {
  const e = await refusal(
    authenticate(bearerReq("/v1/stamps", "not-a-real-service-token-0123456789"), h.deps),
  );
  assert.equal(e.status, 401);
  assert.equal(e.code, "invalid_token");
});

test("a cross-tenant assertion header fails closed (RP caller)", async () => {
  const token = await h.sign(h.rpPayload());
  const res = await dispatch(
    bearerReq("/v1/stamps", token, {
      method: "POST",
      headers: { "x-binding-id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" },
    }),
    h.deps,
  );
  assert.equal(res.status, 403);
  const body = (await res.json()) as { error: string };
  assert.equal(body.error, "cross_tenant_refused");
});

test("a cross-tenant assertion header fails closed (service caller)", async () => {
  const res = await dispatch(
    bearerReq("/v1/stamps", SERVICE_TOKEN, {
      method: "POST",
      headers: { "x-tenancy-binding": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" },
    }),
    h.deps,
  );
  assert.equal(res.status, 403);
  const body = (await res.json()) as { error: string };
  assert.equal(body.error, "cross_tenant_refused");
});

test("a binding header that agrees with the credential is not a refusal", async () => {
  const token = await h.sign(h.rpPayload());
  // The probe rides a still-stubbed door (see the RP-token probe
  // above): past the tenancy check, the stub's 501 is the proof
  // the header was accepted.
  const res = await dispatch(
    bearerReq("/v1/ingestion/github", token, {
      method: "POST",
      headers: { "x-binding-id": BINDING_A },
    }),
    h.deps,
  );
  assert.equal(res.status, 501);
});

test("a per-user Turso DB credential presented as service auth is refused", async () => {
  // libsql/Turso data-plane shape: access claims, no RP iss/aud. Signed
  // with the fixture provider's own key and kid, so the ONLY thing that
  // can refuse it is the §3.3 rule-3 shape check.
  const now = Math.floor(Date.now() / 1000);
  const tursoShaped = await h.sign({ id: "db-1", a: "rw", iat: now, exp: now + 3600 });
  const e = await refusal(authenticate(bearerReq("/v1/stamps", tursoShaped), h.deps));
  assert.equal(e.status, 401);
  assert.equal(e.code, "data_plane_credential_refused");
});

test("an unconfigured deployment fails closed on doors but still answers health", async () => {
  const deps = { config: configFromEnv({}) };
  const door = await dispatch(new Request("https://fleet-hosted.test/v1/stamps", { method: "POST" }), deps);
  assert.equal(door.status, 503);
  const body = (await door.json()) as { error: string };
  assert.equal(body.error, "auth_not_configured");

  const health = await dispatch(new Request("https://fleet-hosted.test/health"), deps);
  assert.equal(health.status, 200);
});
