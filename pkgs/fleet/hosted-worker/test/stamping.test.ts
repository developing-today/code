// Stamping door tests (#12585). Oracles: the issue's acceptance
// (create with no block, a partial block, and a complete block;
// caller-supplied fleet-owned values overwritten from service
// sources; caller-owned values never overwritten; DB row verified by
// query with the full Metadata JSON round-tripping; a forced insert
// failure leaves the GitHub item intact) and design §5's sourcing
// table + §4.3's non-forgeability layers (server-side sourcing, then
// the #12586 signature over the result).

import assert from "node:assert/strict";
import { test } from "node:test";
import { sha256Hex, type AuthDeps } from "../src/auth.ts";
import { configFromEnv } from "../src/config.ts";
import { FixtureGitHubWriter } from "../src/github.ts";
import { parseMetadataBlock } from "../src/metadata.ts";
import { dispatch } from "../src/router.ts";
import { FixtureSessionRegistry } from "../src/sessions.ts";
import { importStampVerifyKeys, verifyStamp, type Provenance } from "../src/stamp-sig.ts";
import { FixtureMetadataStore, FixtureQueueRecordSource } from "../src/stamping.ts";
import { AUDIENCE, BINDING_A, BINDING_B, ISSUER, SERVICE_TOKEN, bearerReq, makeHarness } from "./helpers.ts";

const h = await makeHarness();

const FIXED_MS = Date.parse("2026-11-22T20:00:00.000Z");
const FIXED_ISO = "2026-11-22T20:00:00.000Z";
const STAMP_KEY_ID = "stamp-test-1";
const QUERY_ONLY_TOKEN = "svc-token-query-only-0123456789abcdef";

function b64url(bytes: Uint8Array): string {
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

const stampKeyPair = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
const stampPkcs8 = new Uint8Array(await crypto.subtle.exportKey("pkcs8", stampKeyPair.privateKey));
const stampRawPub = new Uint8Array(await crypto.subtle.exportKey("raw", stampKeyPair.publicKey));

const tenancyFixture = JSON.stringify({
  subjects: [
    { issuer: ISSUER, subject: "user-sub-1", binding_id: BINDING_A, active: true },
    { issuer: ISSUER, subject: "agent-sub-1", binding_id: BINDING_A, active: true },
    { issuer: ISSUER, subject: "user-sub-2", binding_id: BINDING_B, active: true },
  ],
  service_tokens: [
    {
      sha256: await sha256Hex(SERVICE_TOKEN),
      binding_id: BINDING_A,
      routes: ["stamping", "query"],
      active: true,
      label: "pm-service-test",
    },
    {
      sha256: await sha256Hex(QUERY_ONLY_TOKEN),
      binding_id: BINDING_A,
      routes: ["query"],
      active: true,
      label: "query-only",
    },
  ],
});

const config = configFromEnv({
  RP_ISSUER: ISSUER,
  RP_AUDIENCE: AUDIENCE,
  TENANCY_FIXTURE_JSON: tenancyFixture,
  STAMP_SIGNING_KEY_PKCS8: b64url(stampPkcs8),
  STAMP_SIGNING_KEY_ID: STAMP_KEY_ID,
  STAMP_VERIFY_KEYS_JSON: JSON.stringify({ [STAMP_KEY_ID]: b64url(stampRawPub) }),
});
assert.ok(config.stamp.signing, "stamp signing material must parse");
assert.ok(config.stamp.verifyKeys, "stamp verify keys must parse");
const verifyKeys = await importStampVerifyKeys(config.stamp.verifyKeys);

const registry = new FixtureSessionRegistry();
const store = new FixtureMetadataStore();
const writer = new FixtureGitHubWriter();
const queueSource = new FixtureQueueRecordSource();
const deps: AuthDeps = {
  config,
  jwksFetcher: h.deps.jwksFetcher,
  sessions: registry,
  metadataStore: store,
  githubWriter: writer,
  queueRecords: queueSource,
  now: () => FIXED_MS,
};

const tokenA = await h.sign(h.rpPayload());
const tokenB = await h.sign(h.rpPayload({ sub: "user-sub-2" }));

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

interface StampResponse {
  repo: string;
  kind: string;
  number: number;
  url: string;
  body: string;
  metadata: Record<string, any>;
  metadata_inserted: boolean;
  insert_error?: string;
  error?: string;
}

async function createOk(token: string, body: unknown, d: AuthDeps = deps): Promise<StampResponse> {
  const res = await post("/v1/stamps", token, body, d);
  assert.equal(res.status, 201, `create must succeed: ${await res.clone().text()}`);
  return (await res.json()) as StampResponse;
}

async function openSession(token: string, body: unknown): Promise<string> {
  const res = await post("/v1/sessions", token, body);
  assert.equal(res.status, 201);
  return ((await res.json()) as { session_id: string }).session_id;
}

test("create with no block stamps the full set, signs it, and inserts the record", async () => {
  const created = await createOk(tokenA, {
    repo: "developing-today/code",
    kind: "issue",
    title: "No block here",
    body: "Hello world",
  });

  // Required-field defaults for the caller-owned fields.
  assert.equal(created.metadata.pm_meta, 1);
  assert.equal(created.metadata.direct_parent, null);
  assert.deepEqual(created.metadata.links, []);
  assert.deepEqual(created.metadata.lineage, { design: null, roadmap_item: null, priority: null });

  // The §5 stamp set from service sources: service clock, the
  // validated token's binding, this service's identity; nothing else
  // exists for an uncited, non-queue create, so the rest are null.
  const prov = created.metadata.provenance;
  assert.equal(prov.created_at, FIXED_ISO);
  assert.equal(prov.auth_subject, BINDING_A);
  assert.equal(prov.stamped_by, "fleet-hosted@0.1.0");
  assert.equal(prov.session_id, null);
  assert.equal(prov.queue_id, null);
  assert.equal(prov.picked_up_at, null);
  assert.deepEqual(prov.lane_start_issues, []);
  assert.match(prov.stamp_sig, /^v1\.ed25519\.stamp-test-1\./);

  // The body reads back with the block appended after the caller's
  // text, and the stamped provenance verifies.
  assert.ok(created.body.startsWith("Hello world"));
  const reparsed = parseMetadataBlock(created.body);
  assert.deepEqual(reparsed.block, created.metadata);
  assert.deepEqual(await verifyStamp(prov as Provenance, verifyKeys), {
    status: "verified",
    keyId: STAMP_KEY_ID,
  });

  // The DB row, verified by query: the full Metadata JSON
  // round-trips and the extracted keys match.
  assert.equal(created.metadata_inserted, true);
  const row = await store.get("developing-today/code", "issue", created.number);
  assert.ok(row);
  assert.deepEqual(row.metadata, created.metadata);
  assert.equal(row.bindingId, BINDING_A);
  assert.equal(row.extracted.directParent, null);
  assert.equal(row.extracted.sessionId, null);
});

test("create citing a session stamps session, queue, and claim facts from service sources", async () => {
  const sessionId = await openSession(tokenA, { lane: "code-a", model: "model-x" });
  queueSource.seed(BINDING_A, sessionId, {
    queueId: "task-21084",
    scheduledStart: "2026-11-22T19:00:00.000Z",
    pickedUpAt: "2026-11-22T19:05:00.000Z",
    workStartedAt: "2026-11-22T19:06:00.000Z",
    laneStartIssues: ["developing-today/code#12228"],
  });

  const created = await createOk(tokenA, {
    repo: "developing-today/code",
    kind: "issue",
    title: "Session-stamped",
    body: "work",
    session_id: sessionId,
    host: "amd-6",
    worktree: "/home/user/code-12228",
  });
  const prov = created.metadata.provenance;
  assert.equal(prov.session_id, sessionId);
  assert.equal(prov.lane, "code-a");
  assert.equal(prov.model, "model-x");
  assert.equal(prov.queue_id, "task-21084");
  assert.equal(prov.scheduled_start, "2026-11-22T19:00:00.000Z");
  assert.equal(prov.picked_up_at, "2026-11-22T19:05:00.000Z");
  // The queue record's work-start wins over the session-open fallback.
  assert.equal(prov.work_started_at, "2026-11-22T19:06:00.000Z");
  // The session carries no lane-start set, so the queue record's is used.
  assert.deepEqual(prov.lane_start_issues, ["developing-today/code#12228"]);
  assert.equal(prov.host, "amd-6");
  assert.equal(prov.worktree, "/home/user/code-12228");
  assert.deepEqual(prov.workspace_ref, {
    kind: "worktree",
    id: "/home/user/code-12228",
    repo: "developing-today/code",
    branch: null,
    sha: null,
  });
  assert.deepEqual(await verifyStamp(prov as Provenance, verifyKeys), {
    status: "verified",
    keyId: STAMP_KEY_ID,
  });
  const row = await store.get("developing-today/code", "issue", created.number);
  assert.equal(row?.extracted.sessionId, sessionId);
  assert.equal(row?.extracted.queueId, "task-21084");
  assert.equal(row?.extracted.lane, "code-a");
  assert.equal(row?.extracted.host, "amd-6");
});

test("a complete caller block: caller-owned preserved verbatim, forged provenance overwritten", async () => {
  const sessionId = await openSession(tokenA, {
    lane: "code-a",
    model: "model-x",
    lane_start_issues: ["developing-today/code#12585"],
  });
  const callerBlock = {
    pm_meta: 1,
    direct_parent: { repo: "developing-today/code", number: 9081, kind: "design" },
    links: [
      {
        target: { repo: "developing-today/code", number: 9073 },
        relation: "parent",
        status: "active",
        since: "2026-11-22",
      },
    ],
    lineage: { design: "developing-today/code#9081", roadmap_item: null, priority: "normal" },
    provenance: {
      queue_id: "forged-queue",
      session_id: "ses_forged",
      lane: "forged-lane",
      host: "forged-host",
      worktree: null,
      picked_up_at: "1970-01-01T00:00:00.000Z",
      work_started_at: "1970-01-01T00:00:00.000Z",
      created_at: "1970-01-01T00:00:00.000Z",
      model: "forged-model",
      lane_start_issues: ["forged#1"],
      stamp_sig: "v1.ed25519.forged.deadbeef",
    },
  };
  const callerBody = `Intro text\n\n<!-- pm-metadata:start -->\n\`\`\`json\n${JSON.stringify(callerBlock, null, 2)}\n\`\`\`\n<!-- pm-metadata:end -->\n\nTrailing notes`;

  const created = await createOk(tokenA, {
    repo: "developing-today/code",
    kind: "issue",
    title: "Complete block",
    body: callerBody,
    session_id: sessionId,
  });

  // Caller-owned fields are never overwritten.
  assert.deepEqual(created.metadata.direct_parent, callerBlock.direct_parent);
  assert.deepEqual(created.metadata.links, callerBlock.links);
  assert.deepEqual(created.metadata.lineage, callerBlock.lineage);

  // Every forged fleet-owned value is overwritten from service
  // sources (no queue record is seeded for this session → nulls;
  // work_started_at falls back to the session-open time).
  const prov = created.metadata.provenance;
  assert.equal(prov.queue_id, null);
  assert.equal(prov.picked_up_at, null);
  assert.equal(prov.session_id, sessionId);
  assert.equal(prov.lane, "code-a");
  assert.equal(prov.model, "model-x");
  assert.equal(prov.created_at, FIXED_ISO);
  assert.equal(prov.work_started_at, FIXED_ISO);
  assert.deepEqual(prov.lane_start_issues, ["developing-today/code#12585"]);
  assert.ok(!("stamp_sig" in prov) || !String(prov.stamp_sig).includes("forged"));
  assert.deepEqual(await verifyStamp(prov as Provenance, verifyKeys), {
    status: "verified",
    keyId: STAMP_KEY_ID,
  });

  // The caller's surrounding text survives byte-for-byte.
  assert.ok(created.body.startsWith("Intro text\n\n"));
  assert.ok(created.body.endsWith("\nTrailing notes"));
});

test("a partial block takes required-field defaults for the missing caller-owned fields", async () => {
  const callerBody = `Text\n\n<!-- pm-metadata:start -->\n\`\`\`json\n${JSON.stringify({ pm_meta: 1, direct_parent: { repo: "developing-today/code", number: 3447, kind: "epic" } })}\n\`\`\`\n<!-- pm-metadata:end -->`;
  const created = await createOk(tokenA, {
    repo: "developing-today/code",
    kind: "issue",
    title: "Partial block",
    body: callerBody,
  });
  assert.deepEqual(created.metadata.direct_parent, {
    repo: "developing-today/code",
    number: 3447,
    kind: "epic",
  });
  assert.deepEqual(created.metadata.links, []);
  assert.deepEqual(created.metadata.lineage, { design: null, roadmap_item: null, priority: null });
});

test("editing caller-owned fields after stamping still verifies; editing provenance diverges", async () => {
  const created = await createOk(tokenA, {
    repo: "developing-today/code",
    kind: "issue",
    title: "Edit me",
    body: "body",
  });
  const parsed = parseMetadataBlock(created.body);
  assert.ok(parsed.block);

  // Caller-owned fields live outside the signed provenance: editing
  // them cannot invalidate — or forge — the stamp.
  const editedOwned = JSON.parse(JSON.stringify(parsed.block)) as Record<string, any>;
  editedOwned.direct_parent = { repo: "developing-today/code", number: 1, kind: "epic" };
  editedOwned.links = [{ target: { repo: "x/y", number: 2 }, relation: "relates", status: "active" }];
  assert.deepEqual(await verifyStamp(editedOwned.provenance as Provenance, verifyKeys), {
    status: "verified",
    keyId: STAMP_KEY_ID,
  });

  // Provenance is the signed object: any edit diverges.
  const editedProvenance = JSON.parse(JSON.stringify(parsed.block)) as Record<string, any>;
  editedProvenance.provenance.created_at = "1970-01-01T00:00:00.000Z";
  assert.deepEqual(await verifyStamp(editedProvenance.provenance as Provenance, verifyKeys), {
    status: "diverged",
    reason: "signature_mismatch",
  });
});

test("unconfigured signing refuses before any GitHub write or DB insert", async () => {
  const bareWriter = new FixtureGitHubWriter();
  const bareStore = new FixtureMetadataStore();
  const bareDeps: AuthDeps = {
    ...deps,
    config: h.deps.config, // the harness config carries no stamp material
    metadataStore: bareStore,
    githubWriter: bareWriter,
  };
  const res = await post(
    "/v1/stamps",
    tokenA,
    { repo: "developing-today/code", kind: "issue", title: "Nope", body: "x" },
    bareDeps,
  );
  assert.equal(res.status, 503);
  assert.equal(((await res.json()) as { error: string }).error, "stamp_signing_not_configured");
  assert.equal(bareWriter.items.length, 0);
});

test("a create citing another tenancy's session is refused like an unknown session", async () => {
  const sessionId = await openSession(tokenA, { lane: "code-a" });
  const before = writer.items.length;
  const res = await post("/v1/stamps", tokenB, {
    repo: "developing-today/code",
    kind: "issue",
    title: "Cross-tenant",
    body: "x",
    session_id: sessionId,
  });
  assert.equal(res.status, 404);
  assert.equal(((await res.json()) as { error: string }).error, "session_unknown");
  assert.equal(writer.items.length, before);
});

test("a create citing a closed or unknown session is refused with the registry's named errors", async () => {
  const sessionId = await openSession(tokenA, { lane: "code-a" });
  const closed = await post("/v1/sessions/close", tokenA, { session_id: sessionId });
  assert.equal(closed.status, 200);
  const resClosed = await post("/v1/stamps", tokenA, {
    repo: "developing-today/code",
    kind: "issue",
    title: "Closed session",
    body: "x",
    session_id: sessionId,
  });
  assert.equal(resClosed.status, 409);
  assert.equal(((await resClosed.json()) as { error: string }).error, "session_closed");

  const resUnknown = await post("/v1/stamps", tokenA, {
    repo: "developing-today/code",
    kind: "issue",
    title: "Unknown session",
    body: "x",
    session_id: "ses_00000000000000000000000000000000",
  });
  assert.equal(resUnknown.status, 404);
  assert.equal(((await resUnknown.json()) as { error: string }).error, "session_unknown");
});

test("a forced DB-insert failure leaves the GitHub item intact and reports the miss", async () => {
  store.failNextInsert = true;
  const created = await createOk(tokenA, {
    repo: "developing-today/code",
    kind: "issue",
    title: "Insert fails",
    body: "written anyway",
  });
  assert.equal(created.metadata_inserted, false);
  assert.ok(created.insert_error && created.insert_error.length > 0);
  // The GitHub item exists, with the stamped body…
  const item = writer.items.find((i) => i.title === "Insert fails");
  assert.ok(item);
  assert.equal(item.number, created.number);
  assert.ok(item.body.includes("pm-metadata:start"));
  // …and no DB row was written; ingestion (#12590) heals the miss.
  assert.equal(await store.get("developing-today/code", "issue", created.number), null);
});

test("a malformed metadata block in the caller's body refuses the create", async () => {
  const before = writer.items.length;
  const res = await post("/v1/stamps", tokenA, {
    repo: "developing-today/code",
    kind: "issue",
    title: "Bad block",
    body: "Text\n\n<!-- pm-metadata:start -->\n```json\n{not json}\n```\n<!-- pm-metadata:end -->",
  });
  assert.equal(res.status, 400);
  assert.equal(((await res.json()) as { error: string }).error, "invalid_metadata_block");
  assert.equal(writer.items.length, before);
});

test("a service token scoped to stamping creates; one scoped elsewhere is refused at the door", async () => {
  const created = await createOk(SERVICE_TOKEN, {
    repo: "developing-today/code",
    kind: "issue",
    title: "Service create",
    body: "via service token",
  });
  // A service caller with no host claim stamps its service label as
  // the host — the participant that ran the work (design §5).
  assert.equal(created.metadata.provenance.host, "pm-service-test");
  assert.equal(created.metadata.provenance.auth_subject, BINDING_A);

  const res = await post("/v1/stamps", QUERY_ONLY_TOKEN, {
    repo: "developing-today/code",
    kind: "issue",
    title: "Wrong door",
    body: "x",
  });
  assert.equal(res.status, 403);
  assert.equal(((await res.json()) as { error: string }).error, "service_token_route_scope");
});

test("a PR create carries the head/base claims to the writer", async () => {
  const created = await createOk(tokenA, {
    repo: "developing-today/code",
    kind: "pr",
    title: "A PR",
    body: "pr body",
    head: "felix/branch",
    base: "main",
  });
  assert.ok(created.url.includes("/pull/"));
  const item = writer.items.find((i) => i.number === created.number && i.kind === "pr");
  assert.equal(item?.head, "felix/branch");
  assert.equal(item?.base, "main");
  const row = await store.get("developing-today/code", "pr", created.number);
  assert.ok(row);
});

test("an agent caller citing a session beyond its credential is refused", async () => {
  const sessionId = await openSession(tokenA, { lane: "code-a" });
  const agentToken = await h.sign(
    h.rpPayload({
      sub: "agent-sub-1",
      roles: ["agent"],
      lane: "code-a",
      session_id: "ses_11111111111111111111111111111111",
    }),
  );
  const res = await post("/v1/stamps", agentToken, {
    repo: "developing-today/code",
    kind: "issue",
    title: "Beyond credential",
    body: "x",
    session_id: sessionId,
  });
  assert.equal(res.status, 403);
  assert.equal(((await res.json()) as { error: string }).error, "session_beyond_credential");
});
