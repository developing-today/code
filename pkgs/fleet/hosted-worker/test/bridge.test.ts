// Bridge door tests (#14373). Oracle: design §4.8 — the PM service
// API and the console's edge share one bridge path: this door calls
// the bridge Worker over its service binding presenting the PM
// stack's own credential, sends go out as service:pm-stack, and the
// capability document resolves the bridge domain to that same
// binding. A revoked credential is the bridge's plain 404 and stops
// sends here with the refusal named; no binding or no credential is
// a 503 before anything is parsed, like every door's missing
// dependency. The bridge here is a stand-in binding that records
// every request; the credential in it is a fixture, never a real one.

import assert from "node:assert/strict";
import { test } from "node:test";
import { BindingBridgeSurface } from "../src/bridge.ts";
import { buildCapabilitiesDocument } from "../src/capabilities.ts";
import { configFromEnv } from "../src/config.ts";
import { dispatch } from "../src/router.ts";
import { bearerReq, makeHarness } from "./helpers.ts";

const h = await makeHarness();
const token = await h.sign(h.rpPayload());

const PM_TOKEN = "p".repeat(43);

/** The bridge binding, standing in: records requests, answers as the
 * bridge does (its send/agents/messages shapes), and turns the
 * credential away when `refuse` is set — the revoked case. */
function fakeBridge(refuse = false) {
  const calls: { method: string; url: string; body?: unknown }[] = [];
  const binding = {
    async fetch(request: Request): Promise<Response> {
      const text = request.method === "POST" ? await request.text() : "";
      calls.push({
        method: request.method,
        url: request.url,
        ...(text === "" ? {} : { body: JSON.parse(text) }),
      });
      if (refuse) return new Response("Not found\n", { status: 404 });
      if (request.url.endsWith("/send")) {
        return Response.json({ messages: [], fleet: [{ id: 5, agent: "felix-1" }] }, { status: 201 });
      }
      if (request.url.endsWith("/agents")) {
        return Response.json({ agents: [{ name: "felix-1", groups: ["muse"] }] });
      }
      return Response.json({ messages: [{ id: 5, body: "earlier" }] });
    },
  };
  return { calls, binding };
}

function bridgeDeps(refuse = false) {
  const fake = fakeBridge(refuse);
  return { deps: { ...h.deps, bridge: new BindingBridgeSurface(fake.binding, PM_TOKEN) }, calls: fake.calls };
}

test("send: the door delivers through the binding as pm-stack, and the bridge's record comes back", async () => {
  const { deps, calls } = bridgeDeps();
  const res = await dispatch(
    bearerReq("/v1/bridge/send", token, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ to: ["agent:felix-1", "group:muse"], body: "from the service", kind: "command" }),
    }),
    deps,
  );
  assert.equal(res.status, 201);
  assert.deepEqual(((await res.json()) as { fleet: unknown[] }).fleet, [{ id: 5, agent: "felix-1" }]);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].method, "POST");
  assert.equal(calls[0].url, `https://bridge.internal/fleet/${PM_TOKEN}/send`);
  assert.deepEqual(calls[0].body, {
    agent: "pm-stack",
    to: ["agent:felix-1", "group:muse"],
    body: "from the service",
    kind: "command",
  });
});

test("agents and messages: the pipeline reads go to the same binding with the same credential", async () => {
  const { deps, calls } = bridgeDeps();
  const agents = await dispatch(bearerReq("/v1/bridge/agents", token), deps);
  assert.equal(agents.status, 200);
  assert.deepEqual(((await agents.json()) as { agents: unknown[] }).agents, [{ name: "felix-1", groups: ["muse"] }]);
  const log = await dispatch(bearerReq("/v1/bridge/messages?limit=5&order=desc", token), deps);
  assert.equal(log.status, 200);
  assert.deepEqual(((await log.json()) as { messages: unknown[] }).messages, [{ id: 5, body: "earlier" }]);
  assert.deepEqual(
    calls.map((c) => `${c.method} ${c.url}`),
    [
      `GET https://bridge.internal/fleet/${PM_TOKEN}/agents`,
      `GET https://bridge.internal/fleet/${PM_TOKEN}/messages?limit=5&order=desc`,
    ],
  );
});

test("a revoked PM credential stops sends at the door, with the refusal named", async () => {
  const { deps } = bridgeDeps(true);
  const res = await dispatch(
    bearerReq("/v1/bridge/send", token, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ to: ["agent:felix-1"], body: "never sent" }),
    }),
    deps,
  );
  assert.equal(res.status, 502);
  const body = (await res.json()) as { error: string; message: string };
  assert.equal(body.error, "bridge_credential_refused");
  assert.match(body.message, /revoked/);
});

test("unconfigured: no bridge surface is a 503 before the body is read", async () => {
  const res = await dispatch(
    bearerReq("/v1/bridge/send", token, { method: "POST", body: "not json at all" }),
    h.deps,
  );
  assert.equal(res.status, 503);
  assert.equal(((await res.json()) as { error: string }).error, "bridge_unconfigured");
});

test("a bad send is a 400 of the door's own shaping, and never reaches the bridge", async () => {
  const { deps, calls } = bridgeDeps();
  for (const body of [
    { to: [], body: "x" },
    { to: ["not a target!"], body: "x" },
    { to: ["agent:felix-1"], body: "" },
    { to: ["agent:felix-1"], body: "x", kind: "order" },
  ]) {
    const res = await dispatch(
      bearerReq("/v1/bridge/send", token, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      }),
      deps,
    );
    assert.equal(res.status, 400, JSON.stringify(body));
    assert.equal(((await res.json()) as { error: string }).error, "invalid_body");
  }
  assert.equal(calls.length, 0);
});

test("the credential config parses by name only: absent or short is unconfigured", () => {
  assert.equal(configFromEnv({}).bridge, null);
  assert.equal(configFromEnv({ BRIDGE_PM_TOKEN: "short" }).bridge, null);
  assert.deepEqual(configFromEnv({ BRIDGE_PM_TOKEN: PM_TOKEN }).bridge, { token: PM_TOKEN });
});

test("the capability resolves to the same binding: the bridge domain is available, filled by #14373", () => {
  const doc = buildCapabilitiesDocument(h.deps.config) as {
    capabilities: string[];
    domains: { name: string; available: boolean; filling_issue?: string }[];
    doors: { id: string; routes: string[]; state: string; filling_issue: string }[];
  };
  assert.ok(doc.capabilities.includes("bridge"));
  const domain = doc.domains.find((d) => d.name === "bridge");
  assert.equal(domain?.available, true);
  assert.equal(domain?.filling_issue, "#14373");
  const door = doc.doors.find((d) => d.id === "bridge");
  assert.equal(door?.state, "live");
  assert.equal(door?.filling_issue, "#14373");
  assert.deepEqual(door?.routes, ["POST /v1/bridge/send", "GET /v1/bridge/agents", "GET /v1/bridge/messages"]);
});
