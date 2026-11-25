// Queue door tests (#12589). Oracles: the issue's acceptance —
// (1) a packet enqueued through the hosted door is claimed through
// the native door and vice versa, with one shared event timeline;
// (2) two doors claiming concurrently for one eligible item produce
// exactly one claim; (3) an anonymous claim attempt at the hosted
// door is refused while the same packet through the native door is
// unaffected — plus the native broker's own contracts, ported: the
// validator's error strings (validate.rs), the claim guard, the
// completion gate (disposition.rs), and the fallback marker fold
// (fallback.rs). The "native door" in these tests is the same verb
// engine with a native facade env (no binding mark) over the SAME
// store instance — the two doors are peers over one backend, which
// is the design's whole point (§6); the store seam is where a
// production libSQL backend provides the conditional-UPDATE
// atomicity the fixture provides synchronously.

import assert from "node:assert/strict";
import { test } from "node:test";
import { dispatch } from "../src/router.ts";
import { executeQueueVerb, nativeQueueEnv, type QueueEnv } from "../src/queue.ts";
import { FixtureQueueStore, NO_DEP_PROBE, type QueueStore } from "../src/queue-store.ts";
import { StoreQueueRecordSource } from "../src/queue-records.ts";
import { FixtureGitHubWriter } from "../src/github.ts";
import { FixtureSessionRegistry } from "../src/sessions.ts";
import { FixtureMetadataStore } from "../src/stamping.ts";
import {
  fallbackClaim,
  fallbackHeartbeat,
  fallbackRelease,
  fallbackStatus,
  foldClaim,
  parseMarker,
} from "../src/queue-fallback.ts";
import { iso, type JsonObject } from "../src/queue-validate.ts";
import { bearerReq, makeHarness, BINDING_A, BINDING_B, SERVICE_TOKEN } from "./helpers.ts";

const T0 = 1767225600; // 2026-01-01T00:00:00Z
let clock = T0;

function setup(): { store: FixtureQueueStore } {
  clock = T0;
  return { store: new FixtureQueueStore() };
}

function hostedEnv(bindingId: string | null = BINDING_A): QueueEnv {
  return { nowEpoch: () => clock, depProbe: NO_DEP_PROBE, bindingId };
}

function nativeEnv(): QueueEnv {
  return nativeQueueEnv(() => clock);
}

type Packet = ReturnType<typeof fixPacket>;

function fixPacket(issue = 4242, payloadExtra: JsonObject = {}, packetExtra: JsonObject = {}): JsonObject {
  return {
    title: `fix issue ${issue}`,
    priority: 30,
    dedupe_key: `repo-health:v1:fix:developing-today/code:issue:${issue}`,
    payload: {
      schema: "repo-task/v1",
      kind: "small_fix",
      repo: "developing-today/code",
      issue: { number: issue },
      problem: "the thing is broken",
      priority_reason: "it blocks the queue",
      evidence: ["https://example.test/evidence"],
      acceptance: ["the thing works"],
      tests: ["npm test"],
      context_refs: [],
      scope: { include: ["pkgs/fleet"], exclude: ["docs"] },
      risk: { level: "low", flags: [] },
      timebox_minutes: 45,
      dedupe: { checked_at: "2026-01-01T00:00:00Z", basis: "test fixture" },
      ...payloadExtra,
    },
    ...packetExtra,
  };
}

function triagePacket(issue = 777): JsonObject {
  const p = fixPacket(issue);
  p["dedupe_key"] = `repo-health:v1:triage:developing-today/code:issue:${issue}`;
  (p["payload"] as JsonObject)["kind"] = "triage";
  (p["payload"] as JsonObject)["timebox_minutes"] = 20;
  return p;
}

async function enqueue(
  store: QueueStore,
  env: QueueEnv,
  packet: JsonObject,
  extra: JsonObject = {},
): Promise<JsonObject> {
  return await executeQueueVerb(store, "enqueue", { queue: "repo-health", packet, ...extra }, env);
}

async function claim(store: QueueStore, env: QueueEnv, extra: JsonObject = {}): Promise<JsonObject> {
  return await executeQueueVerb(store, "claim", { worker: "worker-a", ...extra }, env);
}

async function verb(store: QueueStore, env: QueueEnv, name: string, req: JsonObject): Promise<JsonObject> {
  return await executeQueueVerb(store, name, req, env);
}

// ---------------------------------------------------------------------------
// The validator (validate.rs port)
// ---------------------------------------------------------------------------

test("validator accepts a well-formed packet and refuses malformed ones with the native strings", async () => {
  const { store } = setup();
  const good = await verb(store, nativeEnv(), "validate", { packet: fixPacket() });
  assert.equal(good["ok"], true);
  assert.deepEqual(good["errors"], []);

  const notObject = await verb(store, nativeEnv(), "validate", { packet: 42 });
  assert.deepEqual(notObject["errors"], ["packet: must be a JSON object"]);

  const noPayload = await verb(store, nativeEnv(), "validate", { packet: { title: "x" } });
  assert.deepEqual(noPayload["errors"], ["packet.payload: required object (repo-task/v1 payload)"]);

  const badKind = fixPacket();
  (badKind["payload"] as JsonObject)["kind"] = "huge_fix";
  const kindErrors = (await verb(store, nativeEnv(), "validate", { packet: badKind }))["errors"] as string[];
  assert.ok(
    kindErrors.includes(
      'payload.kind: must be one of ["ci_investigation", "feature_slice", "pr_shepherd", "second_pass", "small_fix", "triage"]',
    ),
  );

  const offScale = fixPacket(4242, {}, { priority: 15 });
  const priErrors = (await verb(store, nativeEnv(), "validate", { packet: offScale }))["errors"] as string[];
  assert.ok(priErrors.includes("packet.priority: 15 is not on the design scale [10, 20, 30, 40, 50, 70, 90]"));

  const farFuture = fixPacket(4242, {}, { not_before: "2030-01-01T00:00:00Z" });
  const nbErrors = (await verb(store, nativeEnv(), "validate", { packet: farFuture }))["errors"] as string[];
  assert.ok(nbErrors.some((e) => e.includes("more than 366 days in the future")));

  const badDate = fixPacket(4242, {}, { not_before: "2026-02-30T00:00:00Z" });
  const dateErrors = (await verb(store, nativeEnv(), "validate", { packet: badDate }))["errors"] as string[];
  assert.ok(dateErrors.some((e) => e.includes("is not a valid RFC 3339 timestamp")));
});

test("validator enforces dedupe-key family, repo and identity agreement", async () => {
  const { store } = setup();
  // A triage packet carrying a fix key: family not valid for kind.
  const wrongFamily = triagePacket(777);
  wrongFamily["dedupe_key"] = "repo-health:v1:fix:developing-today/code:issue:777";
  const famErrors = (await verb(store, nativeEnv(), "validate", { packet: wrongFamily }))["errors"] as string[];
  assert.ok(
    famErrors.includes(
      `packet.dedupe_key: family 'fix' is not valid for kind 'triage' (allowed: ["maintenance", "triage"])`,
    ),
  );

  // Key issue disagrees with payload issue.
  const wrongIssue = fixPacket(4242);
  wrongIssue["dedupe_key"] = "repo-health:v1:fix:developing-today/code:issue:9999";
  const issueErrors = (await verb(store, nativeEnv(), "validate", { packet: wrongIssue }))["errors"] as string[];
  assert.ok(issueErrors.includes("packet.dedupe_key: issue 9999 in key does not match payload.issue.number 4242"));

  // No key at all: the patterns doc rides the error.
  const noKey = fixPacket();
  delete noKey["dedupe_key"];
  const keyErrors = (await verb(store, nativeEnv(), "validate", { packet: noKey }))["errors"] as string[];
  assert.ok(keyErrors[0].startsWith("packet.dedupe_key: required deterministic key:"));
  assert.ok(keyErrors[0].includes("repo-health:v1:fix:<repo>:issue:<n>"));

  // Leading zeros are not canonical numbers (#5510).
  const leadingZero = fixPacket(4242);
  leadingZero["dedupe_key"] = "repo-health:v1:fix:developing-today/code:issue:04242";
  const lzErrors = (await verb(store, nativeEnv(), "validate", { packet: leadingZero }))["errors"] as string[];
  assert.ok(lzErrors.some((e) => e.includes("matches no design pattern")));
});

test("validator checks the second_pass review block and the feature_slice block", async () => {
  const { store } = setup();
  const sp = fixPacket(4242, { kind: "second_pass" });
  sp["dedupe_key"] = "repo-health:v1:second-pass:developing-today/code:4242:pass-1";
  const spErrors = (await verb(store, nativeEnv(), "validate", { packet: sp }))["errors"] as string[];
  assert.ok(spErrors.includes("payload.review: required object for kind second_pass (design #2025 section 5.2)"));

  (sp["payload"] as JsonObject)["review"] = {
    verdict_note_url: "https://example.test/note",
    gap_list: ["gap one"],
    pass: 2,
    pass_cap: 1,
  };
  const capErrors = (await verb(store, nativeEnv(), "validate", { packet: sp }))["errors"] as string[];
  assert.ok(
    capErrors.includes(
      "payload.review.pass: pass 2 exceeds pass_cap 1 — the termination cap is enforced at enqueue (design #2025 section 5.1)",
    ),
  );
  assert.ok(capErrors.includes("packet.dedupe_key: pass 1 in key does not match payload.review.pass 2"));

  const slice = fixPacket(4242, { kind: "feature_slice" });
  slice["dedupe_key"] = "repo-health:v1:slice:developing-today/code:issue:4242:slice-1";
  const sliceErrors = (await verb(store, nativeEnv(), "validate", { packet: slice }))["errors"] as string[];
  assert.ok(sliceErrors.includes("payload.slice: required object for kind feature_slice (#12233 F2c)"));
});

// ---------------------------------------------------------------------------
// Enqueue: validation envelope, dedupe, revive, work identity
// ---------------------------------------------------------------------------

test("enqueue refuses an invalid packet with the validation envelope, not a throw", async () => {
  const { store } = setup();
  const res = await enqueue(store, hostedEnv(), { title: "x" });
  assert.equal(res["ok"], false);
  assert.equal(res["exit"], 2);
  assert.ok(String(res["error"]).startsWith("packet validation failed ("));
  assert.ok(Array.isArray(res["errors"]));
});

test("enqueue dedupes on (queue, dedupe_key) and revives a terminal predecessor", async () => {
  const { store } = setup();
  const first = await enqueue(store, hostedEnv(), fixPacket());
  assert.equal(first["ok"], true);
  const firstId = (first["row"] as JsonObject)["id"];
  const second = await enqueue(store, hostedEnv(), fixPacket());
  assert.equal((second["row"] as JsonObject)["id"], firstId);
  assert.equal((await store.allTasks()).length, 1);

  // Complete it, then enqueue again: a fresh row revives from it.
  const claimed = await claim(store, nativeEnv(), {});
  assert.equal(claimed["ok"], true);
  const done = await verb(store, nativeEnv(), "complete", {
    task_id: firstId,
    worker: "worker-a",
    result: {
      pr_url: "https://github.com/developing-today/code/pull/999",
      disposition: {
        checked_at: iso(clock),
        source: "test",
        issue: { number: 4242, state: "closed" },
        pulls: [{ number: 999, state: "merged", merged_at: iso(clock) }],
        verdict: "merged_verified",
      },
    },
  });
  assert.equal(done["ok"], true);
  const revived = await enqueue(store, hostedEnv(), fixPacket());
  const revivedRow = revived["row"] as JsonObject;
  assert.notEqual(revivedRow["id"], firstId);
  assert.deepEqual(revived["revived_from"], { task_id: firstId, status: "done" });
  const progress = revivedRow["progress"] as JsonObject;
  assert.equal((progress["revived_from"] as JsonObject)["task_id"], firstId);
});

test("enqueue reports a work-identity collision and honours dedupe_override", async () => {
  const { store } = setup();
  const first = await enqueue(store, hostedEnv(), fixPacket());
  const firstId = (first["row"] as JsonObject)["id"];
  // Same issue + kind, different (dispatch-suffixed) dedupe key:
  // the work identity already has a live task.
  const parallel = fixPacket();
  parallel["dedupe_key"] = "repo-health:v1:fix:developing-today/code:issue:4242:dispatch:0123abcd";
  const collision = await enqueue(store, hostedEnv(), parallel);
  assert.equal((collision["row"] as JsonObject)["id"], firstId);
  const detail = collision["work_identity_collision"] as JsonObject;
  assert.equal(detail["rule"], "open_work_identity");
  assert.equal(detail["identity"], "developing-today/code#4242:fix");
  assert.equal((await store.allTasks()).length, 1);

  // With an override reason, the parallel task is deliberate.
  const overridePacket = fixPacket(4242, { dedupe_override: "parallel verification lane" });
  overridePacket["dedupe_key"] = "repo-health:v1:fix:developing-today/code:issue:4242:dispatch:0123abcd";
  const overridden = await enqueue(store, hostedEnv(), overridePacket);
  assert.notEqual((overridden["row"] as JsonObject)["id"], firstId);
  assert.equal((await store.allTasks()).length, 2);
  const events = (await store.allEvents()).filter((e) => e.kind === "dedupe_override");
  assert.equal(events.length, 1);
  assert.equal((events[0].detail as JsonObject)["reason"], "parallel verification lane");
});

test("enqueue into a shadow queue requires deliberate intent", async () => {
  const { store } = setup();
  const refused = await executeQueueVerb(
    store,
    "enqueue",
    { queue: "repo-health-shadow", packet: fixPacket() },
    hostedEnv(),
  );
  assert.equal(refused["ok"], false);
  assert.equal(refused["exit"], 2);
  const allowed = await executeQueueVerb(
    store,
    "enqueue",
    { queue: "repo-health-shadow", packet: fixPacket(), shadow_intent: true },
    hostedEnv(),
  );
  assert.equal(allowed["ok"], true);
});

test("label enrichment mirrors the issue's labels and the band re-prioritises", async () => {
  const { store } = setup();
  store.seedMirrorLabels("developing-today/code", 4242, ["next-up", "area/fleet"]);
  const res = await enqueue(store, hostedEnv(), fixPacket());
  const row = res["row"] as JsonObject;
  assert.deepEqual((row["payload"] as JsonObject)["labels"], ["next-up", "area/fleet"]);
  // The next-up band (25) outranks the packet's stated 30.
  assert.equal(row["priority"], 25);
});

// ---------------------------------------------------------------------------
// Acceptance 1: cross-door claim, one shared timeline
// ---------------------------------------------------------------------------

test("acceptance 1: hosted-enqueued work is claimed natively and vice versa, one timeline", async () => {
  const { store } = setup();
  // Hosted door enqueues; the native door claims, heartbeats, completes.
  const enq = await enqueue(store, hostedEnv(), fixPacket());
  const id = (enq["row"] as JsonObject)["id"] as number;
  const claimedNative = await claim(store, nativeEnv(), { worker: "native-worker" });
  assert.equal(claimedNative["ok"], true);
  assert.equal((claimedNative["row"] as JsonObject)["worker_id"], "native-worker");
  const hb = await verb(store, hostedEnv(), "heartbeat", { task_id: id, worker: "native-worker" });
  assert.equal(hb["ok"], true, "the hosted door heartbeats the native claim: same worker identity");
  const doneNative = await verb(store, nativeEnv(), "complete", {
    task_id: id,
    worker: "native-worker",
    result: {
      evidence: "commit abc123",
      disposition: {
        checked_at: iso(clock),
        source: "test",
        issue: { number: 4242, state: "closed" },
        pulls: [],
        verdict: "closed_verified",
      },
    },
  });
  assert.equal(doneNative["ok"], true);

  // Native door enqueues; the hosted door claims.
  const enq2 = await enqueue(store, nativeEnv(), fixPacket(5151));
  const id2 = (enq2["row"] as JsonObject)["id"] as number;
  const claimedHosted = await claim(store, hostedEnv(), { worker: "hosted-worker" });
  assert.equal(claimedHosted["ok"], true);
  assert.equal((claimedHosted["row"] as JsonObject)["id"], id2);

  // One shared timeline: the hosted claim's event carries the
  // binding mark; the native claim's carries none.
  const timeline = await verb(store, hostedEnv(), "timeline", { task_id: id2 });
  const rows = (timeline["row"] as JsonObject)["rows"] as JsonObject[];
  const claimedEvent = rows.find((r) => r["kind"] === "claimed");
  assert.ok(claimedEvent);
  assert.equal((claimedEvent["detail"] as JsonObject)["binding_id"], BINDING_A);
  const timeline1 = await verb(store, nativeEnv(), "timeline", { task_id: id });
  const rows1 = (timeline1["row"] as JsonObject)["rows"] as JsonObject[];
  const nativeClaimed = rows1.find((r) => r["kind"] === "claimed");
  assert.ok(nativeClaimed);
  assert.equal((nativeClaimed["detail"] as JsonObject)["binding_id"], undefined);
});

// ---------------------------------------------------------------------------
// Acceptance 2: concurrent claims produce exactly one claim
// ---------------------------------------------------------------------------

test("acceptance 2: two doors claiming concurrently for one item produce exactly one claim", async () => {
  const { store } = setup();
  await enqueue(store, hostedEnv(), fixPacket());
  const first = await claim(store, hostedEnv(), { worker: "door-one" });
  const second = await claim(store, nativeEnv(), { worker: "door-two" });
  assert.equal((first["row"] as JsonObject)["worker_id"], "door-one");
  assert.equal(second["row"], null);
  const claimedEvents = (await store.allEvents()).filter((e) => e.kind === "claimed");
  assert.equal(claimedEvents.length, 1);
});

test("acceptance 2 (race at the seam): a claim that loses the conditional update re-picks and gets nothing", async () => {
  // A store whose conditional update is preempted once by a
  // competing door's claim — the exact interleaving the native
  // backend's conditional UPDATE arbitrates. The engine must not
  // double-claim: its guarded update matches zero rows and the
  // pick finds nothing else.
  class RacingStore extends FixtureQueueStore {
    raced = false;
    override conditionalUpdateTask(
      id: number,
      guard: Partial<import("../src/queue-store.ts").TaskRow>,
      set: Partial<import("../src/queue-store.ts").TaskRow>,
    ): boolean {
      if (!this.raced && set.status === "claimed" && guard.status === "queued") {
        this.raced = true;
        super.conditionalUpdateTask(
          id,
          { status: "queued" },
          {
            status: "claimed",
            worker_id: "racer",
            lease_until: "2999-01-01T00:00:00Z",
          },
        );
      }
      return super.conditionalUpdateTask(id, guard, set);
    }
  }
  const store = new RacingStore();
  clock = T0;
  await enqueue(store, hostedEnv(), fixPacket());
  const res = await claim(store, hostedEnv(), { worker: "door-one" });
  assert.equal(store.raced, true);
  assert.equal(res["row"], null, "the losing door claims nothing");
  assert.equal((await store.allTasks())[0].worker_id, "racer");
});

// ---------------------------------------------------------------------------
// Claim semantics: ordering, attempts, leases, filters, batch
// ---------------------------------------------------------------------------

test("claim picks by priority, then creation order; parked resumes do not burn attempts", async () => {
  const { store } = setup();
  await enqueue(store, nativeEnv(), fixPacket(101, {}, { priority: 50 }));
  await enqueue(store, nativeEnv(), fixPacket(102, {}, { priority: 10 }));
  await enqueue(store, nativeEnv(), fixPacket(103, {}, { priority: 30 }));
  const first = await claim(store, nativeEnv(), {});
  assert.equal((first["row"] as JsonObject)["id"], 2, "priority 10 first");
  assert.equal((first["row"] as JsonObject)["attempts"], 1);
  // Park it and claim again: the parked resume keeps attempts at 1.
  const id = (first["row"] as JsonObject)["id"] as number;
  const parked = await verb(store, nativeEnv(), "park", { task_id: id, worker: "worker-a", delay_seconds: 30 });
  assert.equal(parked["ok"], true);
  clock += 31;
  // A due parked task is picked (by-id claims take queued and
  // handed_off rows only, as natively) — and it outranks the
  // queued rows on priority, still without burning an attempt.
  const resumed = await claim(store, nativeEnv(), {});
  assert.equal((resumed["row"] as JsonObject)["id"], id);
  assert.equal((resumed["row"] as JsonObject)["attempts"], 1);
});

test("an expired lease returns the task and the lapsed holder is told to re-claim", async () => {
  const { store } = setup();
  await enqueue(store, nativeEnv(), fixPacket());
  const claimed = await claim(store, nativeEnv(), { worker: "first", lease_seconds: 60 });
  const id = (claimed["row"] as JsonObject)["id"] as number;
  clock += 61;
  const reclaimed = await claim(store, nativeEnv(), { worker: "second" });
  assert.equal((reclaimed["row"] as JsonObject)["worker_id"], "second");
  const hb = await verb(store, nativeEnv(), "heartbeat", { task_id: id, worker: "first" });
  assert.equal(hb["ok"], false);
  assert.equal(hb["exit"], 5);
  assert.ok(String(hb["error"]).includes("is not claimed by first"));
  // The second worker's own heartbeat names the real holder instead.
  const hb2 = await verb(store, nativeEnv(), "heartbeat", { task_id: id, worker: "third" });
  assert.ok(String(hb2["error"]).includes("it is claimed by second"));
});

test("lease guidance names heartbeat only, and only heartbeat advances the deadline (#14653)", async () => {
  const { store } = setup();
  await enqueue(store, nativeEnv(), fixPacket());
  const claimed = await claim(store, nativeEnv(), { worker: "first", lease_seconds: 60 });
  const id = (claimed["row"] as JsonObject)["id"] as number;
  clock += 61;
  // A claim whose filter matches nothing still runs the expiry
  // sweep, leaving the task queued with first's lease_expired
  // event on record — the state the diagnostic speaks to.
  const none = await claim(store, nativeEnv(), { worker: "second", filter: "kind = triage" });
  assert.equal(none["row"], null);
  const hb = await verb(store, nativeEnv(), "heartbeat", { task_id: id, worker: "first" });
  assert.equal(hb["ok"], false);
  const err = String(hb["error"]);
  assert.ok(err.includes("first's lease expired"), err);
  assert.ok(err.includes("heartbeat to keep a lease"), err);
  assert.ok(!err.includes("or checkpoint"), err);
  // The advertised action is the one that advances the deadline:
  // re-claim, checkpoint (lease_until unchanged), then heartbeat
  // with a longer extension (lease_until advances).
  const reclaimed = await claim(store, nativeEnv(), { worker: "first", lease_seconds: 3600 });
  const before = (reclaimed["row"] as JsonObject)["lease_until"] as string;
  const cp = await verb(store, nativeEnv(), "checkpoint", { task_id: id, worker: "first", stage: "mid" });
  assert.equal(cp["ok"], true);
  assert.equal((cp["row"] as JsonObject)["lease_until"], before);
  const hb2 = await verb(store, nativeEnv(), "heartbeat", { task_id: id, worker: "first", extend_seconds: 7200 });
  assert.equal(hb2["ok"], true);
  assert.ok(String((hb2["row"] as JsonObject)["lease_until"]) > before);
});

test("claim filters, batch claims, and the by-id/task guards", async () => {
  const { store } = setup();
  await enqueue(store, nativeEnv(), fixPacket(201));
  await enqueue(store, nativeEnv(), triagePacket(202));
  const filtered = await claim(store, nativeEnv(), { filter: "kind = triage" });
  assert.equal((filtered["row"] as JsonObject)["id"], 2);
  const badFilter = await claim(store, nativeEnv(), { filter: "kind ~~ triage" });
  assert.equal(badFilter["ok"], false);
  assert.equal(badFilter["exit"], 2);
  assert.ok(String(badFilter["error"]).includes("bad filter"));

  const batch = await claim(store, nativeEnv(), { count: 2, worker: "batcher" });
  assert.equal(batch["claimed"], 1, "one item remains after the filtered claim");
  const batchGuard = await claim(store, nativeEnv(), { count: 2, task_id: 1 });
  assert.equal(batchGuard["ok"], false);
  assert.ok(String(batchGuard["error"]).includes("--count cannot be combined with --task"));
});

test("the claim guard quarantines a stored packet that no longer validates (#5444)", async () => {
  const { store } = setup();
  // Seed a row whose payload is not a valid packet (the native
  // sweep's reason to exist: rows written before validation, or
  // corrupted in place).
  const row = await store.insertTask({
    queue: "repo-health",
    title: "rotten",
    body: null,
    payload: { schema: "repo-task/v1", kind: "small_fix" },
    dedupe_key: null,
    priority: 30,
    status: "queued",
    not_before: null,
    worker_id: null,
    claimed_at: null,
    lease_until: null,
    lane_id: null,
    platform: null,
    attempts: 0,
    max_attempts: 1000,
    claim_provider: null,
    claim_model: null,
    claim_variant: null,
    claim_session_id: null,
    progress: {},
    result: {},
    last_error: null,
    park_kind: null,
    wait_kind: null,
    ready_condition: null,
    required_budget_minutes: null,
    last_checked_at: null,
    check_outcome: null,
    created_at: iso(clock),
    updated_at: iso(clock),
    completed_at: null,
    blocked_deps: null,
    dep_next_recheck_at: null,
    dep_blocked_until: null,
    dep_recheck_seconds: null,
  });
  const res = await claim(store, nativeEnv(), {});
  assert.equal(res["row"], null);
  const after = await store.getTask(row.id);
  assert.equal(after?.status, "blocked");
  assert.ok(after?.last_error?.includes("quarantined by claim path"));
});

// ---------------------------------------------------------------------------
// Lifecycle verbs
// ---------------------------------------------------------------------------

test("heartbeat preserves the later deadline on both the task and lease", async () => {
  const { store } = setup();
  await enqueue(store, nativeEnv(), fixPacket());
  const claimed = await claim(store, nativeEnv(), { lease_seconds: 14400 });
  const id = (claimed["row"] as JsonObject)["id"] as number;
  for (const [elapsed, expected] of [
    [300, 14400],
    [300, 14400],
    [14000, 14900],
  ] as const) {
    clock = T0 + elapsed;
    const heartbeat = await verb(store, nativeEnv(), "heartbeat", { task_id: id, worker: "worker-a" });
    assert.equal((heartbeat["row"] as JsonObject)["lease_until"], iso(T0 + expected));
    assert.equal(store.leaseRow(id)?.lease_until, iso(T0 + expected));
    assert.equal(store.leaseRow(id)?.heartbeat_at, iso(clock));
  }
});

test("heartbeat, checkpoint, park and wake round-trip a claim", async () => {
  const { store } = setup();
  await enqueue(store, nativeEnv(), fixPacket());
  const claimed = await claim(store, nativeEnv(), {});
  const id = (claimed["row"] as JsonObject)["id"] as number;

  const wrongWorker = await verb(store, nativeEnv(), "heartbeat", { task_id: id, worker: "someone-else" });
  assert.equal(wrongWorker["exit"], 5);
  assert.ok(String(wrongWorker["error"]).includes("heartbeat_task: task 1 is not claimed by someone-else"));

  const cp = await verb(store, nativeEnv(), "checkpoint", {
    task_id: id,
    worker: "worker-a",
    stage: "investigating",
    progress: { note: "halfway" },
  });
  assert.equal(cp["ok"], true);
  const cpProgress = (cp["row"] as JsonObject)["progress"] as JsonObject;
  assert.equal((cpProgress["last_checkpoint"] as JsonObject)["stage"], "investigating");
  assert.deepEqual(store.checkpointValue("worker-a", "task:1"), {
    stage: "investigating",
    progress: { note: "halfway" },
    at: iso(clock),
  });

  const parked = await verb(store, nativeEnv(), "park", {
    task_id: id,
    worker: "worker-a",
    delay_seconds: 600,
    reason: "waiting on CI",
  });
  const parkedRow = parked["row"] as JsonObject;
  assert.equal(parkedRow["status"], "parked");
  assert.equal(parkedRow["wait_kind"], "timer");
  assert.equal(parkedRow["not_before"], iso(clock + 600));

  const woken = await verb(store, nativeEnv(), "wake", { task_id: id, worker: "worker-a", reason: "CI done" });
  assert.equal((woken["row"] as JsonObject)["status"], "queued");
  const reclaimed = await claim(store, nativeEnv(), {});
  assert.equal((reclaimed["row"] as JsonObject)["id"], id);
});

test("release requires a reason and returns the task to the queue", async () => {
  const { store } = setup();
  await enqueue(store, nativeEnv(), fixPacket());
  const claimed = await claim(store, nativeEnv(), {});
  const id = (claimed["row"] as JsonObject)["id"] as number;
  const noReason = await verb(store, nativeEnv(), "release", { task_id: id, worker: "worker-a" });
  assert.equal(noReason["exit"], 2);
  assert.ok(String(noReason["error"]).includes("a release requires a reason"));
  const released = await verb(store, nativeEnv(), "release", { task_id: id, worker: "worker-a", reason: "not mine" });
  assert.equal((released["row"] as JsonObject)["status"], "queued");
});

test("fail: transient requeues with backoff, blocked parks the row, fatal terminates; revive restores", async () => {
  const { store } = setup();
  await enqueue(store, nativeEnv(), fixPacket());
  const claimed = await claim(store, nativeEnv(), {});
  const id = (claimed["row"] as JsonObject)["id"] as number;
  const failed = await verb(store, nativeEnv(), "fail", {
    task_id: id,
    worker: "worker-a",
    error: "boom",
    reason_class: "transient",
  });
  const failedRow = failed["row"] as JsonObject;
  assert.equal(failedRow["status"], "queued");
  assert.equal(failedRow["not_before"], iso(clock + 900), "first-attempt backoff is 15 minutes");
  clock += 901;
  const reclaimed = await claim(store, nativeEnv(), {});
  assert.equal((reclaimed["row"] as JsonObject)["attempts"], 2);
  const fatal = await verb(store, nativeEnv(), "fail", {
    task_id: id,
    worker: "worker-a",
    error: "nope",
    reason_class: "fatal",
  });
  assert.equal((fatal["row"] as JsonObject)["status"], "failed");
  const revived = await verb(store, nativeEnv(), "revive", { task_id: id, worker: "worker-a", reason: "retry" });
  const revivedRow = revived["row"] as JsonObject;
  assert.equal(revivedRow["status"], "queued");
  assert.equal(revivedRow["attempts"], 0);
});

test("cancel is idempotent and terminal rows answer unchanged", async () => {
  const { store } = setup();
  await enqueue(store, nativeEnv(), fixPacket());
  const cancelled = await verb(store, nativeEnv(), "cancel", {
    task_id: 1,
    worker: "fleet-monitor",
    reason: "duplicate",
  });
  assert.equal((cancelled["row"] as JsonObject)["status"], "cancelled");
  const again = await verb(store, nativeEnv(), "cancel", { task_id: 1, worker: "fleet-monitor", reason: "duplicate" });
  assert.equal((again["row"] as JsonObject)["status"], "cancelled");
  assert.equal((await store.allEvents()).filter((e) => e.kind === "cancelled").length, 1);
  const missing = await verb(store, nativeEnv(), "cancel", { task_id: 999, worker: "fleet-monitor" });
  assert.equal(missing["exit"], 5);
});

test("block with a task dep rechecks through the store and unblocks when the dep lands", async () => {
  const { store } = setup();
  const depEnq = await enqueue(store, nativeEnv(), triagePacket(555));
  const depId = (depEnq["row"] as JsonObject)["id"] as number;
  await enqueue(store, nativeEnv(), fixPacket(556));
  const claimed = await claim(store, nativeEnv(), { filter: "kind = small_fix" });
  const id = (claimed["row"] as JsonObject)["id"] as number;
  const blocked = await verb(store, nativeEnv(), "block", {
    task_id: id,
    worker: "worker-a",
    blocker: "waiting on the triage",
    reason_class: "dependency",
    deps: [{ kind: "task", task_id: depId }],
  });
  assert.equal((blocked["row"] as JsonObject)["status"], "blocked");
  const early = await verb(store, nativeEnv(), "dep-recheck", { task_id: id });
  assert.equal((early["row"] as JsonObject)["outcome"], "still_blocked");

  // Land the dep (triage exit 'invalid' needs only its note).
  await claim(store, nativeEnv(), { task_id: depId });
  const depDone = await verb(store, nativeEnv(), "complete", {
    task_id: depId,
    worker: "worker-a",
    result: { triage_outcome: "invalid", triage_note: "not a real issue" },
  });
  assert.equal(depDone["ok"], true);
  const recheck = await verb(store, nativeEnv(), "dep-recheck", { task_id: id });
  assert.equal((recheck["row"] as JsonObject)["outcome"], "unblocked");
  assert.equal((await store.getTask(id))?.status, "queued");
});

test("lane assignment, platform limits, and session binding follow the native rules", async () => {
  const { store } = setup();
  await enqueue(store, nativeEnv(), fixPacket());
  const claimed = await claim(store, nativeEnv(), { session_id: "ses_holder1" });
  const id = (claimed["row"] as JsonObject)["id"] as number;

  const assigned = await verb(store, nativeEnv(), "lane", { task_id: id, worker: "worker-a" });
  const ledger = assigned["row"] as JsonObject;
  assert.equal(ledger["assigned_lane"], "direct");

  const noReason = await verb(store, nativeEnv(), "lane", { task_id: id, worker: "worker-a", platform_limit: "muse" });
  assert.equal(noReason["exit"], 4);
  assert.ok(String(noReason["error"]).includes("requires a recorded reason"));

  const bound = await verb(store, nativeEnv(), "bind", { task_id: id, worker: "worker-a", session_id: "ses_bound1" });
  assert.equal(bound["ok"], true);
  assert.equal((bound["row"] as JsonObject)["session_id"], "ses_bound1");
  const rebind = await verb(store, nativeEnv(), "bind", { task_id: id, worker: "worker-a", session_id: "ses_other1" });
  assert.equal(rebind["exit"], 4);
  assert.ok(String(rebind["error"]).includes("already bound to session ses_bound1"));
  const badSession = await verb(store, nativeEnv(), "bind", { task_id: id, worker: "worker-a", session_id: "nope" });
  assert.equal(badSession["exit"], 2);
});

test("a claim transfer moves the claim to the holder's named successor session only", async () => {
  const { store } = setup();
  await enqueue(store, nativeEnv(), fixPacket());
  await claim(store, nativeEnv(), { worker: "holder", session_id: "ses_first" });
  const refused = await claim(store, nativeEnv(), {
    worker: "successor",
    task_id: 1,
    from_session: "ses_wrong",
    session_id: "ses_second",
  });
  assert.equal(refused["ok"], false);
  assert.equal(refused["exit"], 4);
  assert.ok(String(refused["error"]).includes("transfer refused"));
  const moved = await claim(store, nativeEnv(), {
    worker: "successor",
    task_id: 1,
    from_session: "ses_first",
    session_id: "ses_second",
  });
  const row = moved["row"] as JsonObject;
  assert.equal(row["worker_id"], "successor");
  assert.equal((row["progress"] as JsonObject)["session_id"], "ses_second");
});

test("list, show and health report the queue in the native shapes", async () => {
  const { store } = setup();
  await enqueue(store, nativeEnv(), fixPacket(601));
  await enqueue(store, nativeEnv(), fixPacket(602));
  await claim(store, nativeEnv(), {});
  const list = await verb(store, nativeEnv(), "list", { queue: "repo-health" });
  assert.equal(list["total"], 2);
  assert.equal((list["rows"] as JsonObject[]).length, 2);
  const rich = await verb(store, nativeEnv(), "list", { queue: "repo-health", rich: true });
  assert.ok("payload" in (rich["rows"] as JsonObject[])[0]);
  const shown = await verb(store, nativeEnv(), "show", { task_id: 1 });
  assert.equal(shown["ok"], true);
  const missing = await verb(store, nativeEnv(), "show", { task_id: 42 });
  assert.equal(missing["exit"], 5);
  const health = await verb(store, nativeEnv(), "health", {});
  const entry = (health["rows"] as JsonObject[])[0];
  assert.equal(entry["queue"], "repo-health");
  assert.equal(entry["queued"], 1);
  assert.equal(entry["claimed"], 1);
  assert.equal(health["backend"], "hosted-fixture");
});

// ---------------------------------------------------------------------------
// The completion gate (disposition.rs port)
// ---------------------------------------------------------------------------

test("the completion gate refuses fix work without evidence and without a verified disposition", async () => {
  const { store } = setup();
  await enqueue(store, nativeEnv(), fixPacket());
  await claim(store, nativeEnv(), {});
  const bare = await verb(store, nativeEnv(), "complete", { task_id: 1, worker: "worker-a", result: {} });
  assert.equal(bare["exit"], 2);
  assert.ok(String(bare["error"]).includes("fix-class completion requires an evidence reference"));
  assert.equal((await store.getTask(1))?.status, "claimed", "a refused completion leaves the task claimed");

  const noDisposition = await verb(store, nativeEnv(), "complete", {
    task_id: 1,
    worker: "worker-a",
    result: { pr_url: "https://github.com/developing-today/code/pull/999" },
  });
  assert.equal(noDisposition["exit"], 2);
  assert.ok(String(noDisposition["error"]).includes("requires a verified disposition"));

  const stale = await verb(store, nativeEnv(), "complete", {
    task_id: 1,
    worker: "worker-a",
    result: {
      pr_url: "https://github.com/developing-today/code/pull/999",
      disposition: {
        checked_at: iso(clock - 3600),
        source: "test",
        issue: { number: 4242, state: "closed" },
        pulls: [{ number: 999, state: "merged", merged_at: iso(clock - 3600) }],
        verdict: "merged_verified",
      },
    },
  });
  assert.equal(stale["exit"], 2);
  assert.ok(String(stale["error"]).includes("is 3600s old (bound 1800s)"));

  const wrongIssue = await verb(store, nativeEnv(), "complete", {
    task_id: 1,
    worker: "worker-a",
    result: {
      pr_url: "https://github.com/developing-today/code/pull/999",
      disposition: {
        checked_at: iso(clock),
        source: "test",
        issue: { number: 1111, state: "closed" },
        pulls: [{ number: 999, state: "merged", merged_at: iso(clock) }],
        verdict: "merged_verified",
      },
    },
  });
  assert.equal(wrongIssue["exit"], 2);
  assert.ok(String(wrongIssue["error"]).includes("disposition describes issue #1111 but the task is for #4242"));

  const good = await verb(store, nativeEnv(), "complete", {
    task_id: 1,
    worker: "worker-a",
    result: {
      pr_url: "https://github.com/developing-today/code/pull/999",
      disposition: {
        checked_at: iso(clock),
        source: "test",
        issue: { number: 4242, state: "closed" },
        pulls: [{ number: 999, state: "merged", merged_at: iso(clock) }],
        verdict: "merged_verified",
      },
    },
  });
  assert.equal(good["ok"], true);
  assert.equal((await store.getTask(1))?.status, "done");
});

test("triage completion enqueues the fix follow-on once and reuses it live", async () => {
  const { store } = setup();
  await enqueue(store, nativeEnv(), triagePacket(777));
  await claim(store, nativeEnv(), {});
  const done = await verb(store, nativeEnv(), "complete", { task_id: 1, worker: "worker-a", result: {} });
  assert.equal(done["ok"], true);
  const followOnId = done["follow_on_task_id"] as number;
  assert.ok(followOnId > 1);
  const followOn = await store.getTask(followOnId);
  assert.equal(followOn?.dedupe_key, "repo-health:v1:fix:developing-today/code:issue:777");
  assert.equal((followOn?.payload as JsonObject)["kind"], "small_fix");
  assert.equal(followOn?.status, "queued");

  // A second triage of the same issue (revived row) attaches to
  // the same live follow-on instead of enqueueing a twin.
  await enqueue(store, nativeEnv(), triagePacket(777));
  const second = await claim(store, nativeEnv(), { filter: "kind = triage" });
  const secondId = (second["row"] as JsonObject)["id"] as number;
  const done2 = await verb(store, nativeEnv(), "complete", { task_id: secondId, worker: "worker-a", result: {} });
  assert.equal(done2["follow_on_task_id"], followOnId);
});

// ---------------------------------------------------------------------------
// Router-level: acceptance 3, tenancy, scope, configuration, agent ceiling
// ---------------------------------------------------------------------------

// One harness per file (the other suites' pattern): the JWKS
// cache is module-level and keyed by issuer, so a second harness
// with a fresh keypair would poison the first one's tokens.
const h = await makeHarness();

async function routerDeps(store: FixtureQueueStore, writer: FixtureGitHubWriter) {
  const sessions = new FixtureSessionRegistry();
  return {
    deps: {
      ...h.deps,
      sessions,
      metadataStore: new FixtureMetadataStore(),
      queueStore: store,
      queueRecords: new StoreQueueRecordSource(store, sessions),
      githubComments: writer,
      now: () => clock * 1000,
    },
    sign: h.sign,
    rpPayload: h.rpPayload,
  };
}

function post(path: string, token: string | null, body?: unknown, headers: Record<string, string> = {}): Request {
  return new Request(`https://fleet-hosted.test${path}`, {
    method: "POST",
    headers: {
      ...(token ? { authorization: `Bearer ${token}` } : {}),
      "content-type": "application/json",
      ...headers,
    },
    body: typeof body === "string" ? body : JSON.stringify(body ?? {}),
  });
}

test("acceptance 3: an anonymous claim at the hosted door is refused; the native door is unaffected", async () => {
  const { store } = setup();
  const writer = new FixtureGitHubWriter();
  const { deps } = await routerDeps(store, writer);
  const anon = await dispatch(post("/v1/queue/claim", null, { worker: "anon" }), deps);
  assert.equal(anon.status, 401);
  // The same work through the native door never needed a credential.
  const enq = await enqueue(store, nativeEnv(), fixPacket());
  assert.equal(enq["ok"], true);
  const claimed = await claim(store, nativeEnv(), { worker: "native-worker" });
  assert.equal(claimed["ok"], true);
});

test("the hosted door answers envelopes at 200 and refuses cross-tenant and out-of-scope callers", async () => {
  const { store } = setup();
  const writer = new FixtureGitHubWriter();
  const { deps, sign, rpPayload } = await routerDeps(store, writer);
  const token = await sign(rpPayload());

  const enq = await dispatch(post("/v1/queue/enqueue", token, { queue: "repo-health", packet: fixPacket() }), deps);
  assert.equal(enq.status, 200);
  assert.equal(((await enq.json()) as JsonObject)["ok"], true);

  const crossTenant = await dispatch(
    post("/v1/queue/claim", token, { worker: "w" }, { "x-binding-id": BINDING_B }),
    deps,
  );
  assert.equal(crossTenant.status, 403);
  assert.equal(((await crossTenant.json()) as { error: string }).error, "cross_tenant_refused");

  const outOfScope = await dispatch(post("/v1/queue/claim", SERVICE_TOKEN, { worker: "w" }), deps);
  assert.equal(outOfScope.status, 403);
  assert.equal(((await outOfScope.json()) as { error: string }).error, "service_token_route_scope");

  const malformed = await dispatch(post("/v1/queue/enqueue", token, "not json"), deps);
  assert.equal(malformed.status, 400);
  assert.equal(((await malformed.json()) as { error: string }).error, "invalid_body");
});

test("an unconfigured queue store fails closed 503, and an agent's lane assertion is capped", async () => {
  const { store } = setup();
  const writer = new FixtureGitHubWriter();
  const { deps, sign, rpPayload } = await routerDeps(store, writer);
  const token = await sign(rpPayload());
  // The shared harness's auth config with no queue store wired.
  const unconfigured = await dispatch(post("/v1/queue/health", token, {}), h.deps);
  assert.equal(unconfigured.status, 503);
  assert.equal(((await unconfigured.json()) as { error: string }).error, "queue_store_not_configured");

  const agentToken = await sign(
    rpPayload({ sub: "agent-sub-1", roles: ["agent"], lane: "code-a", session_id: "sess-123" }),
  );
  const beyond = await dispatch(post("/v1/queue/claim", agentToken, { worker: "w", lane: "code-b" }), deps);
  assert.equal(beyond.status, 403);
  assert.equal(((await beyond.json()) as { error: string }).error, "lane_beyond_credential");
  const within = await dispatch(post("/v1/queue/claim", agentToken, { worker: "w", lane: "code-a" }), deps);
  assert.equal(within.status, 200);
});

// ---------------------------------------------------------------------------
// The QueueRecordSource seam (#12585's, wired here)
// ---------------------------------------------------------------------------

test("StoreQueueRecordSource reads the queue's claim record for the caller's own session only", async () => {
  const { store } = setup();
  const sessions = new FixtureSessionRegistry();
  await sessions.insert({
    sessionId: "ses_bound42",
    bindingId: BINDING_A,
    lane: "code-a",
    model: null,
    openedAt: iso(clock),
    closedAt: null,
    laneStartIssues: [],
  });
  await enqueue(store, nativeEnv(), fixPacket());
  await claim(store, nativeEnv(), { worker: "worker-a", session_id: "ses_bound42" });
  const source = new StoreQueueRecordSource(store, sessions);
  const record = await source.recordForSession(BINDING_A, "ses_bound42");
  assert.ok(record);
  assert.equal(record.queueId, "task-1");
  assert.equal(record.pickedUpAt, iso(clock));
  assert.equal(record.workStartedAt, iso(clock), "the session_bound event's time");
  assert.equal(record.scheduledStart, null, "the claim cleared the scheduled start");
  assert.deepEqual(record.laneStartIssues, []);
  // Another tenancy's session, and an unknown one, both answer
  // null — no existence oracle.
  assert.equal(await source.recordForSession(BINDING_B, "ses_bound42"), null);
  assert.equal(await source.recordForSession(BINDING_A, "ses_nope"), null);
});

// ---------------------------------------------------------------------------
// The fallback-claim protocol (fallback.rs port)
// ---------------------------------------------------------------------------

test("fallback markers parse strictly and the fold contests, supersedes and releases", async () => {
  const claimLine = "<!-- fleet-claim v1 kind=claim worker=felix ts=2026-01-01T00:00:00Z -->";
  assert.deepEqual(parseMarker(`progress note\n${claimLine}\nmore`), {
    kind: "claim",
    worker: "felix",
    outcome: null,
    ts: T0,
  });
  // A release without an outcome, and a claim carrying one, are
  // not markers at all.
  assert.equal(parseMarker("<!-- fleet-claim v1 kind=release worker=felix ts=2026-01-01T00:00:00Z -->"), null);
  assert.equal(
    parseMarker("<!-- fleet-claim v1 kind=claim worker=felix outcome=done ts=2026-01-01T00:00:00Z -->"),
    null,
  );
  assert.equal(parseMarker("<!-- fleet-claim v1 kind=claim worker=bad worker! ts=2026-01-01T00:00:00Z -->"), null);

  const lease = 1800;
  const events = [
    { kind: "claim" as const, worker: "a", outcome: null, ts: T0 },
    { kind: "claim" as const, worker: "b", outcome: null, ts: T0 + 60 },
  ];
  const contested = foldClaim(events, T0 + 120, lease);
  assert.equal(contested.state, "contested");
  // The contest ages out after a lease of challenger silence; the
  // holder's own silence then makes the claim stale, claimable.
  const aged = foldClaim(events, T0 + 60 + lease + 1, lease);
  assert.equal(aged.state, "stale");
  const released = foldClaim(
    [...events, { kind: "release" as const, worker: "a", outcome: "free", ts: T0 + 200 }],
    T0 + 300,
    lease,
  );
  assert.equal(released.state, "released");
});

test("the fallback verbs run the whole claim lifecycle over the comment surface", async () => {
  const { store } = setup();
  void store;
  const writer = new FixtureGitHubWriter();
  const ctx = { surface: writer, nowEpoch: () => clock };
  const req = { repo: "developing-today/code", number: 12589 };

  const noSummary = await fallbackRelease(ctx, { ...req, worker: "felix", outcome: "done" });
  assert.equal(noSummary.status, 400);

  const claimed = await fallbackClaim(ctx, { ...req, worker: "felix" });
  assert.equal(claimed.status, 200);
  assert.equal(claimed.body["claimed"], true);
  assert.deepEqual(writer.labelsFor("developing-today/code", 12589), ["queue/claimed", "work/assigned"]);

  const rival = await fallbackClaim(ctx, { ...req, worker: "other" });
  assert.equal(rival.status, 409);
  assert.equal(rival.body["reason"], "live_claim");

  const status = await fallbackStatus(ctx, req);
  const claimJson = status.body["claim"] as JsonObject;
  assert.equal(claimJson["state"], "live");
  assert.equal(claimJson["worker"], "felix");

  const rivalHeartbeat = await fallbackHeartbeat(ctx, { ...req, worker: "other" });
  assert.equal(rivalHeartbeat.status, 409);
  assert.equal(rivalHeartbeat.body["reason"], "held_by_other");
  const heartbeat = await fallbackHeartbeat(ctx, { ...req, worker: "felix", note: "halfway" });
  assert.equal(heartbeat.status, 200);

  const released = await fallbackRelease(ctx, {
    ...req,
    worker: "felix",
    outcome: "free",
    summary: "handing back",
  });
  assert.equal(released.status, 200);
  assert.deepEqual(writer.labelsFor("developing-today/code", 12589), ["queue/queued"]);

  // After the lease lapses, a stale claim is supersedeable.
  await fallbackClaim(ctx, { ...req, worker: "felix" });
  clock += 31 * 60;
  const supersede = await fallbackClaim(ctx, { ...req, worker: "other" });
  assert.equal(supersede.status, 200);
  assert.equal(supersede.body["superseded"], "felix");
});

test("the fallback verbs answer through the router with the native result shapes", async () => {
  const { store } = setup();
  const writer = new FixtureGitHubWriter();
  const { deps, sign, rpPayload } = await routerDeps(store, writer);
  const token = await sign(rpPayload());
  const claimed = await dispatch(
    post("/v1/queue/fallback/claim", token, { repo: "developing-today/code", number: 12589, worker: "felix" }),
    deps,
  );
  assert.equal(claimed.status, 200);
  assert.equal(((await claimed.json()) as JsonObject)["claimed"], true);
  const rival = await dispatch(
    post("/v1/queue/fallback/claim", token, { repo: "developing-today/code", number: 12589, worker: "other" }),
    deps,
  );
  assert.equal(rival.status, 409);
  assert.equal(((await rival.json()) as JsonObject)["reason"], "live_claim");
});
