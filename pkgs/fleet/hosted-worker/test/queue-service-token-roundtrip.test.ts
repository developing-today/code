// Queue-scoped service-token round-trip (#14942 CF-2). Oracle: the
// cutover's recorded auth route — a queue-scoped service token over
// the tenancy directory alone, no RP issuer — must serve the whole
// broker write lifecycle through the HTTP door (enqueue, claim,
// heartbeat, park, wake, re-claim, complete), with the
// credential-resolved binding stamped into the timeline. The
// in-process round-trip pins that contract against the fixture
// store; the deploy-gated round-trip replays it against the live
// door when FLEET_HOSTED_BASE_URL + FLEET_HOSTED_QUEUE_TOKEN are
// set, and skips otherwise (never a failure). The live run works
// in its own queue (door-roundtrip-smoke) with a run-unique
// synthetic issue and always leaves the task terminal, so a run
// is a record, not residue.

import assert from "node:assert/strict";
import { test } from "node:test";
import { dispatch } from "../src/router.ts";
import { FixtureQueueStore, type QueueStore } from "../src/queue-store.ts";
import { StoreQueueRecordSource } from "../src/queue-records.ts";
import { FixtureGitHubWriter } from "../src/github.ts";
import { FixtureSessionRegistry } from "../src/sessions.ts";
import { FixtureMetadataStore } from "../src/stamping.ts";
import type { JsonObject } from "../src/queue-validate.ts";
import { bearerReq, makeHarness, BINDING_A, SERVICE_TOKEN, QUEUE_SERVICE_TOKEN } from "./helpers.ts";

const QUEUE = "door-roundtrip-smoke";
const WORKER = "roundtrip-probe";

const h = await makeHarness();

function fixPacket(issue: number): JsonObject {
  return {
    title: `round-trip probe issue ${issue}`,
    priority: 30,
    dedupe_key: `repo-health:v1:fix:developing-today/code:issue:${issue}`,
    payload: {
      schema: "repo-task/v1",
      kind: "small_fix",
      repo: "developing-today/code",
      issue: { number: issue },
      problem: "the probe task is synthetic",
      priority_reason: "it proves the cutover auth route",
      evidence: ["https://example.test/evidence"],
      acceptance: ["the round-trip completes"],
      tests: ["npm test"],
      context_refs: [],
      scope: { include: ["pkgs/fleet"], exclude: ["docs"] },
      risk: { level: "low", flags: [] },
      timebox_minutes: 45,
      dedupe: { checked_at: "2026-01-01T00:00:00Z", basis: "test fixture" },
    },
  };
}

function post(path: string, token: string, body: unknown): Request {
  const req = bearerReq(path, token, { method: "POST" });
  return new Request(req, {
    body: JSON.stringify(body ?? {}),
    headers: { ...Object.fromEntries(req.headers), "content-type": "application/json" },
  });
}

async function call(
  deps: Parameters<typeof dispatch>[1],
  path: string,
  token: string,
  body: unknown,
): Promise<{ status: number; body: JsonObject }> {
  const res = await dispatch(post(path, token, body), deps);
  return { status: res.status, body: (await res.json()) as JsonObject };
}

async function routerDeps(store: QueueStore) {
  const sessions = new FixtureSessionRegistry();
  return {
    ...h.deps,
    sessions,
    metadataStore: new FixtureMetadataStore(),
    queueStore: store,
    queueRecords: new StoreQueueRecordSource(store, sessions),
    githubComments: new FixtureGitHubWriter(),
    now: () => 1767225600000,
  };
}

function disposition(issue: number): JsonObject {
  return {
    evidence: "round-trip probe run (#14942)",
    disposition: {
      checked_at: "2026-01-01T00:00:00Z",
      source: "test",
      issue: { number: issue, state: "closed" },
      pulls: [],
      verdict: "closed_verified",
    },
  };
}

test("the queue-scoped service token runs the full lifecycle through the HTTP door", async () => {
  const deps = await routerDeps(new FixtureQueueStore());
  const issue = 900101;

  const enq = await call(deps, "/v1/queue/enqueue", QUEUE_SERVICE_TOKEN, { queue: QUEUE, packet: fixPacket(issue) });
  assert.equal(enq.status, 200);
  assert.equal(enq.body["ok"], true);
  const id = (enq.body["row"] as JsonObject)["id"] as number;
  assert.equal(typeof id, "number");

  const claimed = await call(deps, "/v1/queue/claim", QUEUE_SERVICE_TOKEN, { queue: QUEUE, worker: WORKER });
  assert.equal(claimed.status, 200);
  assert.equal(claimed.body["ok"], true);
  assert.equal((claimed.body["row"] as JsonObject)["id"], id);
  assert.equal((claimed.body["row"] as JsonObject)["worker_id"], WORKER);

  const hb = await call(deps, "/v1/queue/heartbeat", QUEUE_SERVICE_TOKEN, { task_id: id, worker: WORKER });
  assert.equal(hb.status, 200);
  assert.equal(hb.body["ok"], true);

  const parked = await call(deps, "/v1/queue/park", QUEUE_SERVICE_TOKEN, {
    task_id: id,
    worker: WORKER,
    delay_seconds: 600,
    reason: "probe park",
  });
  assert.equal(parked.status, 200);
  assert.equal(parked.body["ok"], true);
  assert.equal((parked.body["row"] as JsonObject)["status"], "parked");

  const woken = await call(deps, "/v1/queue/wake", QUEUE_SERVICE_TOKEN, {
    task_id: id,
    worker: WORKER,
    reason: "probe wake",
  });
  assert.equal(woken.status, 200);
  assert.equal((woken.body["row"] as JsonObject)["status"], "queued");

  const reclaimed = await call(deps, "/v1/queue/claim", QUEUE_SERVICE_TOKEN, { queue: QUEUE, worker: WORKER });
  assert.equal(reclaimed.body["ok"], true);
  assert.equal((reclaimed.body["row"] as JsonObject)["id"], id);

  const done = await call(deps, "/v1/queue/complete", QUEUE_SERVICE_TOKEN, {
    task_id: id,
    worker: WORKER,
    result: disposition(issue),
  });
  assert.equal(done.status, 200);
  assert.equal(done.body["ok"], true);

  // One shared timeline: the service caller's claimed event carries
  // the credential-resolved binding, never a client assertion.
  const timeline = await call(deps, "/v1/queue/timeline", QUEUE_SERVICE_TOKEN, { task_id: id });
  assert.equal(timeline.body["ok"], true);
  const rows = (timeline.body["row"] as JsonObject)["rows"] as JsonObject[];
  const kinds = rows.map((r) => r["kind"]);
  assert.ok(kinds.includes("claimed"), `timeline carries the claim, got ${JSON.stringify(kinds)}`);
  assert.ok(kinds.includes("completed"), `timeline carries the completion, got ${JSON.stringify(kinds)}`);
  const claimedEvent = rows.find((r) => r["kind"] === "claimed");
  assert.equal((claimedEvent?.["detail"] as JsonObject)?.["binding_id"], BINDING_A);

  const health = await call(deps, "/v1/queue/health", QUEUE_SERVICE_TOKEN, {});
  assert.equal(health.status, 200);
  assert.equal(health.body["ok"], true);
});

test("a token scoped to other doors is refused on queue writes", async () => {
  const deps = await routerDeps(new FixtureQueueStore());
  const res = await call(deps, "/v1/queue/enqueue", SERVICE_TOKEN, { queue: QUEUE, packet: fixPacket(900102) });
  assert.equal(res.status, 403);
  assert.equal((res.body as { error: string }).error, "service_token_route_scope");
});

// ---------------------------------------------------------------------------
// Deploy-gated live round-trip (the CF-2 cutover verification): the
// same lifecycle against the deployed door over HTTPS with the
// queue-scoped service token. With FLEET_HOSTED_BASE_URL +
// FLEET_HOSTED_QUEUE_TOKEN unset this is a skip, never a failure.
// ---------------------------------------------------------------------------

interface LiveConfig {
  baseUrl: string;
  token: string;
}

function liveConfig(): LiveConfig | null {
  const baseUrl = process.env["FLEET_HOSTED_BASE_URL"];
  const token = process.env["FLEET_HOSTED_QUEUE_TOKEN"];
  if (!baseUrl || !token) return null;
  return { baseUrl: baseUrl.replace(/\/$/, ""), token };
}

async function liveCall(cfg: LiveConfig, path: string, body: unknown): Promise<JsonObject> {
  const res = await fetch(`${cfg.baseUrl}${path}`, {
    method: "POST",
    headers: { authorization: `Bearer ${cfg.token}`, "content-type": "application/json" },
    body: JSON.stringify(body ?? {}),
  });
  assert.equal(res.status, 200, `${path} answers 200, got ${res.status}: ${await res.text()}`);
  return (await res.json()) as JsonObject;
}

test("live door: the queue-scoped token round-trips enqueue/claim/park/complete", async (t) => {
  const cfg = liveConfig();
  if (!cfg) {
    t.skip("no live door configured: FLEET_HOSTED_BASE_URL / FLEET_HOSTED_QUEUE_TOKEN unset");
    return;
  }
  const issue = 800000 + (Date.now() % 100000);
  let id = 0;
  let done = false;
  try {
    const enq = await liveCall(cfg, "/v1/queue/enqueue", { queue: QUEUE, packet: fixPacket(issue) });
    assert.equal(enq["ok"], true);
    id = ((enq["row"] as JsonObject)["id"] as number) ?? 0;
    assert.ok(id > 0);

    const claimed = await liveCall(cfg, "/v1/queue/claim", { queue: QUEUE, worker: WORKER });
    assert.equal(claimed["ok"], true);
    assert.equal((claimed["row"] as JsonObject)["id"] as number, id);

    const parked = await liveCall(cfg, "/v1/queue/park", {
      task_id: id,
      worker: WORKER,
      delay_seconds: 60,
      reason: "live probe park",
    });
    assert.equal(parked["ok"], true);
    assert.equal((parked["row"] as JsonObject)["status"] as string, "parked");

    const woken = await liveCall(cfg, "/v1/queue/wake", { task_id: id, worker: WORKER, reason: "live probe wake" });
    assert.equal((woken["row"] as JsonObject)["status"] as string, "queued");

    const reclaimed = await liveCall(cfg, "/v1/queue/claim", { queue: QUEUE, worker: WORKER });
    assert.equal(reclaimed["ok"], true);

    const completed = await liveCall(cfg, "/v1/queue/complete", {
      task_id: id,
      worker: WORKER,
      result: disposition(issue),
    });
    assert.equal(completed["ok"], true);
    done = true;

    const timeline = await liveCall(cfg, "/v1/queue/timeline", { task_id: id });
    const rows = ((timeline["row"] as JsonObject)["rows"] as JsonObject[]).map((r) => r["kind"]);
    assert.ok(
      rows.includes("claimed") && rows.includes("completed"),
      `live timeline is shared, got ${JSON.stringify(rows)}`,
    );
  } finally {
    // A run is a record, not residue: never leave the probe task
    // hanging in a live queue.
    if (id > 0 && !done) {
      await liveCall(cfg, "/v1/queue/cancel", { task_id: id, worker: WORKER, reason: "probe cleanup" });
    }
  }
});
