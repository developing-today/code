// Thin TS CLI tests (#12588). Oracles: the issue's acceptance —
// (1) from a Codespace-class environment with no fleet binary:
// open a session, create a stamped issue, query it back by
// session, take and claim a queue item, end to end through the
// service; (2) a hand-crafted request asserting fleet-owned
// provenance fields produces a stamp that ignores them — and
// design §3.2: the CLI holds no authority, so its create request
// is a fixed allowlist of caller-owned fields and claims, with no
// timestamp, queue field, or provenance key anywhere in it.
// The end-to-end tests run the CLI against the real router
// in-process (the smoke harness's shape), so the chain is the
// service's own doors answering, not a mock of them.

import assert from "node:assert/strict";
import { test } from "node:test";
import type { AuthDeps } from "../src/auth.ts";
import { resolveCallerToken, runCli, type CliDeps } from "../src/cli.ts";
import { configFromEnv } from "../src/config.ts";
import { FixtureGitHubWriter } from "../src/github.ts";
import { FixtureQueueStore } from "../src/queue-store.ts";
import { dispatch } from "../src/router.ts";
import { InMemorySessionStateStore, type SessionStateStore } from "../src/session-resolution.ts";
import { FixtureSessionRegistry } from "../src/sessions.ts";
import { importStampVerifyKeys, verifyStamp, type Provenance } from "../src/stamp-sig.ts";
import { FixtureMetadataStore, FixtureQueueRecordSource } from "../src/stamping.ts";
import { parseFlowSyncCliArgs } from "../src/sync-cli.ts";
import { AUDIENCE, BINDING_A, ISSUER, bearerReq, makeHarness } from "./helpers.ts";

const h = await makeHarness();

const FIXED_MS = Date.parse("2026-11-22T20:00:00.000Z");
const FIXED_ISO = "2026-11-22T20:00:00.000Z";
const STAMP_KEY_ID = "stamp-test-1";

function b64url(bytes: Uint8Array): string {
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

const stampKeyPair = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
const stampPkcs8 = new Uint8Array(await crypto.subtle.exportKey("pkcs8", stampKeyPair.privateKey));
const stampRawPub = new Uint8Array(await crypto.subtle.exportKey("raw", stampKeyPair.publicKey));

const config = configFromEnv({
  RP_ISSUER: ISSUER,
  RP_AUDIENCE: AUDIENCE,
  TENANCY_FIXTURE_JSON: JSON.stringify({
    subjects: [
      { issuer: ISSUER, subject: "user-sub-1", binding_id: BINDING_A, active: true },
      { issuer: ISSUER, subject: "agent-sub-1", binding_id: BINDING_A, active: true },
    ],
    service_tokens: [],
  }),
  STAMP_SIGNING_KEY_PKCS8: b64url(stampPkcs8),
  STAMP_SIGNING_KEY_ID: STAMP_KEY_ID,
  STAMP_VERIFY_KEYS_JSON: JSON.stringify({ [STAMP_KEY_ID]: b64url(stampRawPub) }),
});
assert.ok(config.stamp.signing, "stamp signing material must parse");
assert.ok(config.stamp.verifyKeys, "stamp verify keys must parse");
const verifyKeys = await importStampVerifyKeys(config.stamp.verifyKeys);

interface World {
  deps: AuthDeps;
  registry: FixtureSessionRegistry;
  metaStore: FixtureMetadataStore;
  writer: FixtureGitHubWriter;
  queueStore: FixtureQueueStore;
}

function makeWorld(): World {
  const registry = new FixtureSessionRegistry();
  const metaStore = new FixtureMetadataStore();
  const writer = new FixtureGitHubWriter();
  const queueStore = new FixtureQueueStore();
  return {
    registry,
    metaStore,
    writer,
    queueStore,
    deps: {
      config,
      jwksFetcher: h.deps.jwksFetcher,
      sessions: registry,
      metadataStore: metaStore,
      githubWriter: writer,
      githubItems: writer,
      queueRecords: new FixtureQueueRecordSource(),
      queueStore,
      now: () => FIXED_MS,
    },
  };
}

function makeCli(world: World, token: string | null, store: SessionStateStore): CliDeps {
  return {
    serviceUrl: "https://fleet-hosted.test",
    token,
    fetch: (req) => dispatch(req, world.deps),
    env: () => null,
    store,
    readFile: () => {
      throw new Error("no files in this test");
    },
    hostname: () => "codespace-test",
    cwd: () => "/work/cli-test",
  };
}

function fixPacket(issue: number): Record<string, unknown> {
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
    },
  };
}

async function enqueue(world: World, token: string, issue: number): Promise<void> {
  const res = await dispatch(
    bearerReq("/v1/queue/enqueue", token, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ queue: "repo-health", packet: fixPacket(issue) }),
    }),
    world.deps,
  );
  assert.equal(res.status, 200);
  const envelope = (await res.json()) as { ok?: boolean };
  assert.equal(envelope.ok, true, "enqueue must succeed");
}

// --- Token carriage ---------------------------------------------------

test("token resolution is explicit > env > stored, whitespace absent", async () => {
  const envWith = (value: string | null) => (name: string) => (name === "FLEET_HOSTED_TOKEN" ? value : null);
  assert.equal(resolveCallerToken({ explicit: "tok-a", env: envWith("tok-b"), stored: "tok-c" }), "tok-a");
  assert.equal(resolveCallerToken({ explicit: "  ", env: envWith("tok-b"), stored: "tok-c" }), "tok-b");
  assert.equal(resolveCallerToken({ env: envWith(null), stored: " tok-c " }), "tok-c");
  assert.equal(resolveCallerToken({ env: envWith(""), stored: null }), null);
});

test("no token refuses before any service call", async () => {
  let calls = 0;
  const deps: CliDeps = {
    ...makeCli(makeWorld(), null, new InMemorySessionStateStore()),
    fetch: async () => {
      calls++;
      throw new Error("must not be called");
    },
  };
  const out = await runCli(["session", "open"], deps);
  assert.equal(out.exitCode, 1);
  assert.equal(calls, 0);
  assert.equal((JSON.parse(out.stderr) as { error: string }).error, "missing_token");
});

// --- Argument contract (#3352) -----------------------------------------

test("the declared command line refuses anything unaccounted for", async () => {
  const deps = makeCli(makeWorld(), "tok", new InMemorySessionStateStore());
  const bad: string[][] = [
    [],
    ["bogus"],
    ["session"],
    ["session", "open", "--bogus", "x"],
    ["session", "resume", "extra"],
    ["create"],
    ["create", "issue", "--title", "t"],
    ["create", "issue", "--repo", "o/r", "--title", "t", "--head", "x"],
    ["create", "issue", "--repo", "o/r", "--title", "t", "--body", "a", "--body-file", "f"],
    ["create", "issue", "--repo", "o/r", "--title", "t", "--link", "bogus"],
    ["next", "--worker", "w", "--count", "9"],
    ["next", "--task-id", "3", "--worker", "w"],
    ["claim", "--worker", "w", "--task-id", "0"],
    ["sync"],
    ["sync", "bogus"],
  ];
  for (const argv of bad) {
    const out = await runCli(argv, deps);
    assert.equal(out.exitCode, 1, argv.join(" "));
    assert.equal((JSON.parse(out.stderr) as { error: string }).error, "invalid_cli", argv.join(" "));
  }
  // Intake with no worker anywhere refuses by name.
  const out = await runCli(["next"], deps);
  assert.equal(out.exitCode, 1);
  assert.equal((JSON.parse(out.stderr) as { error: string }).error, "invalid_cli");
});

// --- The no-authority request shape ------------------------------------

interface RecordedCall {
  path: string;
  body: Record<string, unknown>;
}

function recordingDeps(canned: Record<string, unknown>, calls: RecordedCall[]): CliDeps {
  return {
    serviceUrl: "https://fleet-hosted.test",
    token: "tok",
    fetch: async (req) => {
      calls.push({
        path: new URL(req.url).pathname,
        body: JSON.parse(await req.text()) as Record<string, unknown>,
      });
      return new Response(JSON.stringify(canned), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    },
    env: () => null,
    store: new InMemorySessionStateStore(),
    readFile: () => {
      throw new Error("no files in this test");
    },
    hostname: () => "codespace-test",
    cwd: () => "/work/cli-test",
  };
}

test("the create request is caller-owned fields and claims only — no fleet-owned keys", async () => {
  const calls: RecordedCall[] = [];
  const deps = recordingDeps({ number: 7 }, calls);
  const forgedBody =
    'Text\n\n<!-- pm-metadata:start -->\n```json\n{"pm_meta": 1, "provenance": {"created_at": "1970-01-01T00:00:00.000Z", "queue_id": "forged-q", "stamp_sig": "v1.ed25519.fake.x"}}\n```\n<!-- pm-metadata:end -->';
  const out = await runCli(
    [
      "create",
      "issue",
      "--repo",
      "developing-today/code",
      "--title",
      "Forged provenance attempt",
      "--body",
      forgedBody,
      "--direct-parent",
      "developing-today/code#9081",
    ],
    deps,
  );
  assert.equal(out.exitCode, 0, out.stderr);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, "/v1/stamps");
  // The fixed allowlist, exactly: no created_at, no queue fields,
  // no provenance object, no session (none carried in this test).
  assert.deepEqual(Object.keys(calls[0].body).sort(), ["body", "host", "kind", "repo", "title", "worktree"]);
  // The caller's body passes through untouched (the service, not
  // the CLI, replaces its provenance) — with the flag's
  // caller-owned field merged into its block.
  const sentBody = calls[0].body["body"] as string;
  assert.ok(sentBody.includes("1970-01-01T00:00:00.000Z"), "body is not laundered by the CLI");
  assert.ok(sentBody.includes('"direct_parent"'), "the flag's caller-owned field is assembled in");
});

test("query flags reach the door in its own wire shape", async () => {
  const calls: RecordedCall[] = [];
  const canned = { filter: {}, count: 0, results: [] };
  const deps = recordingDeps(canned, calls);
  const out = await runCli(
    ["query", "--session", "ses_abc123", "--direct-parent", "developing-today/code#9081", "--lane", "xaltha"],
    deps,
  );
  assert.equal(out.exitCode, 0, out.stderr);
  assert.equal(calls[0].path, "/v1/query");
  assert.deepEqual(calls[0].body, {
    session_id: "ses_abc123",
    direct_parent: "developing-today/code#9081",
    lane: "xaltha",
  });
  assert.deepEqual(JSON.parse(out.stdout), canned);
});

test("sync flow flags build the door's request document via its own parser", async () => {
  const calls: RecordedCall[] = [];
  const deps = recordingDeps({ flow: "verdict", posted: true }, calls);
  const flags = [
    "--flow",
    "verdict",
    "--repo",
    "developing-today/code",
    "--number",
    "12588",
    "--marker",
    "verdict/pass",
    "--body",
    "looks right",
  ];
  const out = await runCli(["sync", "flow", ...flags], deps);
  assert.equal(out.exitCode, 0, out.stderr);
  assert.equal(calls[0].path, "/v1/sync/queue-github");
  assert.deepEqual(calls[0].body, JSON.parse(JSON.stringify(parseFlowSyncCliArgs(flags))) as Record<string, unknown>);
});

// --- Acceptance, end to end through the service -------------------------

test("acceptance: open, create, query back by session, take and claim — no fleet binary", async () => {
  const world = makeWorld();
  const token = await h.sign(h.rpPayload());
  await enqueue(world, token, 4242);
  await enqueue(world, token, 4243);
  const store = new InMemorySessionStateStore();
  const cli = makeCli(world, token, store);

  // Open a session: the service mints the id; the CLI carries it
  // into its stored state for later bare invocations.
  const opened = await runCli(
    [
      "session",
      "open",
      "--lane",
      "xaltha",
      "--model",
      "test-model",
      "--lane-start-issue",
      "developing-today/code#9081",
    ],
    cli,
  );
  assert.equal(opened.exitCode, 0, opened.stderr);
  const record = JSON.parse(opened.stdout) as Record<string, unknown>;
  const sessionId = record["session_id"] as string;
  assert.match(sessionId, /^ses_[A-Za-z0-9]+$/);
  assert.equal(record["lane"], "xaltha");
  assert.equal(record["binding_id"], BINDING_A);
  assert.equal(store.read(), sessionId);

  // Create a stamped issue citing the carried session.
  const created = await runCli(
    [
      "create",
      "issue",
      "--repo",
      "developing-today/code",
      "--title",
      "Made by the thin CLI",
      "--body",
      "No fleet binary was involved.",
      "--direct-parent",
      "developing-today/code#9081",
    ],
    cli,
  );
  assert.equal(created.exitCode, 0, created.stderr);
  const createdDoc = JSON.parse(created.stdout) as {
    number: number;
    metadata: { direct_parent: unknown; provenance: Provenance };
  };
  assert.deepEqual(createdDoc.metadata.direct_parent, { repo: "developing-today/code", number: 9081 });
  const prov = createdDoc.metadata.provenance;
  assert.equal(prov.session_id, sessionId);
  assert.equal(prov.created_at, FIXED_ISO);
  assert.equal(prov.auth_subject, BINDING_A);
  assert.equal(prov.host, "codespace-test");
  assert.equal(prov.worktree, "/work/cli-test");
  assert.deepEqual(await verifyStamp(prov, verifyKeys), { status: "verified", keyId: STAMP_KEY_ID });

  // Query it back by session.
  const queried = await runCli(["query", "--session", sessionId], cli);
  assert.equal(queried.exitCode, 0, queried.stderr);
  const queryDoc = JSON.parse(queried.stdout) as { count: number; results: { number: number }[] };
  assert.equal(queryDoc.count, 1);
  assert.equal(queryDoc.results[0].number, createdDoc.number);

  // Take and claim a queue item: the composite claims, assigns the
  // lane ledger, and binds the carried session to the task.
  const next = await runCli(["next", "--worker", "xaltha", "--lane", "xaltha", "--queue", "repo-health"], cli);
  assert.equal(next.exitCode, 0, next.stderr);
  const nextDoc = JSON.parse(next.stdout) as {
    claim: { ok: boolean; row: { id: number } };
    intake: { task_id: number; lane: { ok: boolean }; bind: { ok: boolean } | null }[];
  };
  assert.equal(nextDoc.claim.ok, true);
  assert.equal(nextDoc.intake.length, 1);
  assert.equal(nextDoc.intake[0].lane.ok, true);
  assert.equal(nextDoc.intake[0].bind?.ok, true);
  const ledger = await world.queueStore.getLedger(nextDoc.claim.row.id);
  assert.equal(ledger?.session_id, sessionId);

  // The bare claim verb takes the second item on its own.
  const claimed = await runCli(["claim", "--worker", "xaltha", "--queue", "repo-health"], cli);
  assert.equal(claimed.exitCode, 0, claimed.stderr);
  assert.equal((JSON.parse(claimed.stdout) as { ok: boolean }).ok, true);

  // A fresh CLI invocation in the same workspace recovers the
  // session from stored state and resumes it.
  const fresh = makeCli(world, token, store);
  const resumed = await runCli(["session", "resume"], fresh);
  assert.equal(resumed.exitCode, 0, resumed.stderr);
  assert.equal((JSON.parse(resumed.stdout) as { session_id: string }).session_id, sessionId);

  // Close it; a create citing the closed session is refused by the
  // service with its named error, surfaced by the CLI unchanged.
  const closed = await runCli(["session", "close"], cli);
  assert.equal(closed.exitCode, 0, closed.stderr);
  assert.notEqual((JSON.parse(closed.stdout) as { closed_at: unknown }).closed_at, null);
  const refused = await runCli(
    ["create", "issue", "--repo", "developing-today/code", "--title", "x", "--session-id", sessionId],
    cli,
  );
  assert.equal(refused.exitCode, 1);
  assert.equal((JSON.parse(refused.stderr) as { error: string }).error, "session_closed");
});

test("acceptance: a hand-crafted request asserting fleet-owned provenance is stamped over", async () => {
  const world = makeWorld();
  const token = await h.sign(h.rpPayload());
  const cli = makeCli(world, token, new InMemorySessionStateStore());
  const forgedBody =
    'Text\n\n<!-- pm-metadata:start -->\n```json\n{"pm_meta": 1, "provenance": {"created_at": "1970-01-01T00:00:00.000Z", "queue_id": "forged-q", "session_id": "ses_forged", "stamp_sig": "v1.ed25519.fake.invalid"}}\n```\n<!-- pm-metadata:end -->';
  const created = await runCli(
    ["create", "issue", "--repo", "developing-today/code", "--title", "Forged", "--body", forgedBody],
    cli,
  );
  assert.equal(created.exitCode, 0, created.stderr);
  const prov = (JSON.parse(created.stdout) as { metadata: { provenance: Provenance } }).metadata.provenance;
  // Every asserted fleet-owned value is ignored: the stamp is the
  // service's clock, no queue, no cited session, a real signature.
  assert.equal(prov.created_at, FIXED_ISO);
  assert.equal(prov.queue_id, null);
  assert.equal(prov.session_id, null);
  assert.deepEqual(await verifyStamp(prov, verifyKeys), { status: "verified", keyId: STAMP_KEY_ID });
});

test("a stored session the registry no longer knows is the named stale error", async () => {
  const world = makeWorld();
  const token = await h.sign(h.rpPayload());
  const store = new InMemorySessionStateStore();
  store.write("ses_deadbeefdeadbeef");
  const cli = makeCli(world, token, store);
  for (const argv of [
    ["session", "resume"],
    ["create", "issue", "--repo", "o/r", "--title", "t"],
  ]) {
    const out = await runCli(argv, cli);
    assert.equal(out.exitCode, 1, argv.join(" "));
    assert.equal((JSON.parse(out.stderr) as { error: string }).error, "stale_session_state", argv.join(" "));
  }
});
