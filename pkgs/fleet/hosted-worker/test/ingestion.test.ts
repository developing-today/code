// Ingestion door tests (#12590). Oracles: the issue's acceptance —
// (1) an item created outside the service (web UI / plain gh)
// appears in the metadata DB via the webhook path; (2) with the
// listener paused, the sweeper catches the same item with the
// same check result; (3) a quickly-merged PR gets its Metadata
// completed retroactively by the sweeper; (4) duplicate delivery
// is idempotent (one DB row, no duplicate queue items) — plus
// design §7.3/§16's rules as implemented in src/ingestion.ts: the
// signature is the delivery's credential (503 unconfigured, 401
// unverified, before any parsing); the check reads the item fresh
// through the item source, never the payload; a malformed block
// fails closed; insufficient Metadata is forwarded to triage once;
// the #12585 create-path heal (metadata_inserted: false → the
// first check stores the row).

import assert from "node:assert/strict";
import { test } from "node:test";
import { sha256Hex, type AuthDeps } from "../src/auth.ts";
import { configFromEnv } from "../src/config.ts";
import { FixtureGitHubWriter } from "../src/github.ts";
import { TRIAGE_QUEUE, triageDedupeKey } from "../src/ingestion.ts";
import { METADATA_END, METADATA_START, parseMetadataBlock, renderBodyWithBlock } from "../src/metadata.ts";
import { FixtureQueueStore } from "../src/queue-store.ts";
import { dispatch } from "../src/router.ts";
import { FixtureSessionRegistry } from "../src/sessions.ts";
import { importStampVerifyKeys, verifyStamp, type Provenance } from "../src/stamp-sig.ts";
import { FixtureMetadataStore, FixtureQueueRecordSource } from "../src/stamping.ts";
import {
  AUDIENCE,
  BINDING_A,
  BINDING_B,
  ISSUER,
  WEBHOOK_SECRET,
  bearerReq,
  makeHarness,
} from "./helpers.ts";

const h = await makeHarness();

const REPO = "developing-today/code";
const REPO2 = "developing-today/other";
const FIXED_MS = Date.parse("2026-11-22T21:00:00.000Z");
const STAMP_KEY_ID = "stamp-test-1";
const INGEST_TOKEN = "svc-token-ingestion-0123456789abcdef";
const QUERY_ONLY_TOKEN = "svc-token-query-only-0123456789abcde";

function b64url(bytes: Uint8Array): string {
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

const stampKeyPair = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
const stampPkcs8 = new Uint8Array(await crypto.subtle.exportKey("pkcs8", stampKeyPair.privateKey));
const stampRawPub = new Uint8Array(await crypto.subtle.exportKey("raw", stampKeyPair.publicKey));

const tenancyFixture = JSON.stringify({
  subjects: [{ issuer: ISSUER, subject: "user-sub-1", binding_id: BINDING_A, active: true }],
  service_tokens: [
    { sha256: await sha256Hex(INGEST_TOKEN), binding_id: BINDING_A, routes: ["ingestion"], active: true, label: "ingestion-test" },
    { sha256: await sha256Hex(QUERY_ONLY_TOKEN), binding_id: BINDING_A, routes: ["query"], active: true, label: "query-only" },
  ],
});

interface World {
  deps: AuthDeps;
  store: FixtureMetadataStore;
  writer: FixtureGitHubWriter;
  queueStore: FixtureQueueStore;
}

function makeWorld(envOverrides: Record<string, string | undefined> = {}): World {
  const store = new FixtureMetadataStore();
  const writer = new FixtureGitHubWriter();
  const queueStore = new FixtureQueueStore();
  const config = configFromEnv({
    RP_ISSUER: ISSUER,
    RP_AUDIENCE: AUDIENCE,
    TENANCY_FIXTURE_JSON: tenancyFixture,
    GITHUB_WEBHOOK_SECRET: WEBHOOK_SECRET,
    STAMP_SIGNING_KEY_PKCS8: b64url(stampPkcs8),
    STAMP_SIGNING_KEY_ID: STAMP_KEY_ID,
    STAMP_VERIFY_KEYS_JSON: JSON.stringify({ [STAMP_KEY_ID]: b64url(stampRawPub) }),
    ...envOverrides,
  });
  return {
    deps: {
      config,
      jwksFetcher: h.deps.jwksFetcher,
      sessions: new FixtureSessionRegistry(),
      metadataStore: store,
      githubWriter: writer,
      githubItems: writer,
      githubEditor: writer,
      queueRecords: new FixtureQueueRecordSource(),
      queueStore,
      now: () => FIXED_MS,
    },
    store,
    writer,
    queueStore,
  };
}

const tokenA = await h.sign(h.rpPayload());

/** A §12 block as a native (other-edition) writer would have
 * stamped it: parent resolved, tenancy = the stamped subject. */
function authoredBlock(repo: string, overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    pm_meta: 1,
    direct_parent: { repo, number: 100, kind: "epic" },
    links: [],
    lineage: { design: null, roadmap_item: null, priority: null },
    provenance: {
      queue_id: null,
      scheduled_start: null,
      picked_up_at: null,
      work_started_at: null,
      created_at: "2026-11-20T10:00:00.000Z",
      session_id: null,
      lane: null,
      host: null,
      worktree: null,
      model: null,
      lane_start_issues: [],
      workspace_ref: null,
      auth_subject: BINDING_A,
      stamped_by: "native-fleet@test",
    },
    ...overrides,
  };
}

function bodyWithBlock(text: string, block: Record<string, unknown>): string {
  return renderBodyWithBlock(parseMetadataBlock(text), block);
}

async function signDelivery(secret: string, rawBody: string): Promise<string> {
  const key = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const sig = new Uint8Array(await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(rawBody)));
  let hex = "";
  for (const b of sig) hex += b.toString(16).padStart(2, "0");
  return `sha256=${hex}`;
}

interface DeliverOpts {
  event?: string;
  delivery?: string;
  /** null → send no signature header; a string → sign with it. */
  secret?: string | null;
  rawBody?: string;
}

async function deliver(world: World, payload: unknown, opts: DeliverOpts = {}): Promise<Response> {
  const rawBody = opts.rawBody ?? JSON.stringify(payload);
  const headers: Record<string, string> = {
    "content-type": "application/json",
    "x-github-event": opts.event ?? "issues",
    "x-github-delivery": opts.delivery ?? "delivery-1",
  };
  const secret = opts.secret === undefined ? WEBHOOK_SECRET : opts.secret;
  if (secret !== null) headers["x-hub-signature-256"] = await signDelivery(secret, rawBody);
  return dispatch(
    new Request("https://fleet-hosted.test/v1/ingestion/github", {
      method: "POST",
      headers,
      body: rawBody,
    }),
    world.deps,
  );
}

function issuePayload(repo: string, number: number, action = "opened"): unknown {
  return { action, repository: { full_name: repo }, issue: { number } };
}

async function sweep(world: World, token: string, body?: unknown): Promise<Response> {
  return dispatch(
    bearerReq("/v1/ingestion/sweep", token, {
      method: "POST",
      headers: { "content-type": "application/json" },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    }),
    world.deps,
  );
}

interface CheckResult {
  outcome: string;
  block?: string;
  binding_id?: string;
  sufficient?: boolean;
  queue?: { action: string; task_id?: number; dedupe_key?: string };
  writeback?: string;
  stamp?: string;
  state?: string | null;
  merged?: boolean;
  error?: string;
}

// --- Acceptance 1: the webhook path stores an outside-created item.

test("webhook: an item created outside the service lands in the DB via the webhook path", async () => {
  const world = makeWorld();
  world.writer.seedItem({
    repo: REPO,
    kind: "issue",
    number: 7,
    title: "Created in the web UI",
    body: bodyWithBlock("Filed from the web UI.", authoredBlock(REPO)),
    state: "open",
    labels: [],
    createdAt: "2026-11-20T10:00:00.000Z",
  });
  const res = await deliver(world, issuePayload(REPO, 7));
  assert.equal(res.status, 200, await res.clone().text());
  const result = (await res.json()) as CheckResult;
  assert.equal(result.outcome, "inserted");
  assert.equal(result.block, "authored");
  assert.equal(result.binding_id, BINDING_A);
  assert.equal(result.sufficient, true);
  assert.equal(result.queue?.action, "not_warranted");
  const row = await world.store.get(REPO, "issue", 7);
  assert.ok(row, "the item must be in the metadata DB");
  assert.equal(row.bindingId, BINDING_A);
  assert.equal((row.metadata["direct_parent"] as { number: number }).number, 100);
});

// --- The #12585 heal: a create whose insert failed is stored by
// the first check that sees the item.

test("webhook heals a create whose metadata insert failed (metadata_inserted: false)", async () => {
  const world = makeWorld();
  world.store.failNextInsert = true;
  const createRes = await dispatch(
    bearerReq("/v1/stamps", tokenA, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ repo: REPO, kind: "issue", title: "Heal me", body: "Created via the door" }),
    }),
    world.deps,
  );
  assert.equal(createRes.status, 201, await createRes.clone().text());
  const created = (await createRes.json()) as { number: number; metadata_inserted: boolean };
  assert.equal(created.metadata_inserted, false);
  assert.equal(await world.store.get(REPO, "issue", created.number), null);

  const res = await deliver(world, issuePayload(REPO, created.number));
  assert.equal(res.status, 200, await res.clone().text());
  const result = (await res.json()) as CheckResult;
  assert.equal(result.outcome, "inserted");
  const row = await world.store.get(REPO, "issue", created.number);
  assert.ok(row, "ingestion must heal the missing row");
  assert.equal(row.bindingId, BINDING_A);
});

// --- Acceptance 4: duplicate delivery is idempotent, at both the
// DB layer and the queue layer.

test("duplicate delivery: one DB row, unchanged second result, one triage task", async () => {
  const world = makeWorld();
  // An insufficient item: no block, no parent marks → generated
  // block, triage forward.
  world.writer.seedItem({
    repo: REPO,
    kind: "issue",
    number: 8,
    title: "No block, no parent",
    body: "Just prose.",
    state: "open",
    labels: [],
    createdAt: "2026-11-21T09:00:00.000Z",
  });
  const first = await deliver(world, issuePayload(REPO, 8), { delivery: "d-1" });
  assert.equal(first.status, 200, await first.clone().text());
  const r1 = (await first.json()) as CheckResult;
  assert.equal(r1.outcome, "inserted");
  assert.equal(r1.block, "ascertained");
  assert.equal(r1.binding_id, "unattributed");
  assert.equal(r1.sufficient, false);
  assert.equal(r1.queue?.action, "enqueued");
  const key = triageDedupeKey(REPO, 8);
  const tasks = world.queueStore.tasksByDedupe(TRIAGE_QUEUE, key);
  assert.equal(tasks.length, 1);
  assert.equal((tasks[0].payload as { kind: string }).kind, "triage");

  const second = await deliver(world, issuePayload(REPO, 8), { delivery: "d-2" });
  assert.equal(second.status, 200, await second.clone().text());
  const r2 = (await second.json()) as CheckResult;
  assert.equal(r2.outcome, "unchanged");
  assert.equal(r2.queue?.action, "already_enqueued");
  assert.equal(world.queueStore.tasksByDedupe(TRIAGE_QUEUE, key).length, 1);
  const rows = await world.store.list("unattributed", {});
  assert.equal(rows.filter((r) => r.number === 8).length, 1);
});

// --- Delivery authentication: the signature is the credential.

test("webhook refuses a bad signature and a missing signature before any parsing", async () => {
  const world = makeWorld();
  const bad = await deliver(world, issuePayload(REPO, 7), { secret: "the-wrong-secret" });
  assert.equal(bad.status, 401);
  assert.equal(((await bad.json()) as { error: string }).error, "webhook_signature_invalid");

  const missing = await deliver(world, issuePayload(REPO, 7), { secret: null });
  assert.equal(missing.status, 401);
  assert.equal(((await missing.json()) as { error: string }).error, "webhook_signature_missing");
});

test("webhook fails closed 503 when no webhook secret is configured", async () => {
  const world = makeWorld({ GITHUB_WEBHOOK_SECRET: undefined });
  const res = await deliver(world, issuePayload(REPO, 7));
  assert.equal(res.status, 503);
  assert.equal(((await res.json()) as { error: string }).error, "webhook_not_configured");
});

test("webhook refuses a verified delivery that does not name its item", async () => {
  const world = makeWorld();
  const garbage = await deliver(world, null, { rawBody: "{not json" });
  assert.equal(garbage.status, 400);
  assert.equal(((await garbage.json()) as { error: string }).error, "invalid_webhook_payload");

  const noNumber = await deliver(world, { action: "opened", repository: { full_name: REPO }, issue: {} });
  assert.equal(noNumber.status, 400);
  assert.equal(((await noNumber.json()) as { error: string }).error, "invalid_webhook_payload");
});

test("webhook acknowledges events and actions the check has no business with", async () => {
  const world = makeWorld();
  const ping = await deliver(world, { zen: "hi" }, { event: "ping" });
  assert.equal(ping.status, 200);
  assert.equal(((await ping.json()) as { outcome: string }).outcome, "ignored");

  const assigned = await deliver(world, issuePayload(REPO, 7, "assigned"));
  assert.equal(assigned.status, 200);
  assert.equal(((await assigned.json()) as { outcome: string }).outcome, "ignored");
  assert.equal(await world.store.get(REPO, "issue", 7), null);
});

test("webhook 404s a delivery for an item the read path cannot see", async () => {
  const world = makeWorld();
  const res = await deliver(world, issuePayload(REPO, 999));
  assert.equal(res.status, 404);
  assert.equal(((await res.json()) as { error: string }).error, "item_not_visible");
});

test("webhook fails closed 503 when the deployment has no metadata store", async () => {
  const world = makeWorld();
  const deps: AuthDeps = { ...world.deps, metadataStore: undefined };
  const rawBody = JSON.stringify(issuePayload(REPO, 7));
  const res = await dispatch(
    new Request("https://fleet-hosted.test/v1/ingestion/github", {
      method: "POST",
      headers: {
        "x-github-event": "issues",
        "x-hub-signature-256": await signDelivery(WEBHOOK_SECRET, rawBody),
      },
      body: rawBody,
    }),
    deps,
  );
  assert.equal(res.status, 503);
  assert.equal(((await res.json()) as { error: string }).error, "metadata_store_not_configured");
});

// --- Ascertainment: block-less items get a generated block.

test("a block-less item with a resolvable primary-parent label is ascertained, signed, and written back", async () => {
  const world = makeWorld();
  world.writer.seedItem({
    repo: REPO,
    kind: "issue",
    number: 100,
    title: "The roadmap epic",
    body: "Epic body.",
    state: "open",
    labels: ["epic", "epic/roadmap"],
    createdAt: "2026-11-01T00:00:00.000Z",
  });
  world.writer.seedItem({
    repo: REPO,
    kind: "issue",
    number: 11,
    title: "Child of the epic",
    body: "No block yet.",
    state: "open",
    labels: ["primary-parent/roadmap"],
    createdAt: "2026-11-21T09:30:00.000Z",
  });
  const res = await deliver(world, issuePayload(REPO, 11));
  assert.equal(res.status, 200, await res.clone().text());
  const result = (await res.json()) as CheckResult;
  assert.equal(result.outcome, "inserted");
  assert.equal(result.block, "ascertained");
  assert.equal(result.writeback, "written");
  assert.equal(result.stamp, "signed");
  assert.equal(result.sufficient, true);
  assert.equal(result.queue?.action, "not_warranted");

  const row = await world.store.get(REPO, "issue", 11);
  assert.ok(row);
  assert.deepEqual(row.metadata["direct_parent"], { repo: REPO, number: 100, kind: "epic" });

  // The write-back landed on the body, and the generated
  // provenance's signature verifies under the service's key.
  const item = await world.writer.getItem(REPO, "issue", 11);
  assert.ok(item?.body.includes(METADATA_START));
  const written = parseMetadataBlock(item.body);
  assert.ok(written.block);
  const config = world.deps.config;
  assert.ok(config.stamp.verifyKeys);
  const keys = await importStampVerifyKeys(config.stamp.verifyKeys);
  const verdict = await verifyStamp(written.block["provenance"] as Provenance, keys);
  assert.equal(verdict.status, "verified");
});

test("a refused write-back still stores the row and names the refusal", async () => {
  const world = makeWorld();
  world.writer.failBodyEdits = true;
  world.writer.seedItem({
    repo: REPO,
    kind: "issue",
    number: 12,
    title: "No write access",
    body: "No block.",
    state: "open",
    labels: [],
    createdAt: "2026-11-21T09:00:00.000Z",
  });
  const res = await deliver(world, issuePayload(REPO, 12));
  assert.equal(res.status, 200, await res.clone().text());
  const result = (await res.json()) as CheckResult;
  assert.equal(result.block, "ascertained");
  assert.equal(result.writeback, "refused");
  assert.ok(await world.store.get(REPO, "issue", 12), "the DB row is the record even when the body edit is refused");
});

test("an authored block already queue-tracked is sufficient without a parent", async () => {
  const world = makeWorld();
  const block = authoredBlock(REPO, {
    direct_parent: null,
    provenance: {
      ...(authoredBlock(REPO)["provenance"] as Record<string, unknown>),
      queue_id: "21101",
    },
  });
  world.writer.seedItem({
    repo: REPO,
    kind: "issue",
    number: 13,
    title: "Queue-tracked",
    body: bodyWithBlock("Tracked.", block),
    state: "open",
    labels: [],
    createdAt: "2026-11-20T10:00:00.000Z",
  });
  const res = await deliver(world, issuePayload(REPO, 13));
  assert.equal(res.status, 200, await res.clone().text());
  const result = (await res.json()) as CheckResult;
  assert.equal(result.sufficient, true);
  assert.equal(result.queue?.action, "not_warranted");
  assert.equal(world.queueStore.tasksByDedupe(TRIAGE_QUEUE, triageDedupeKey(REPO, 13)).length, 0);
});

// --- Malformed input fails closed, per door idiom.

test("a malformed block refuses the webhook with 400 and becomes a per-item error in a sweep", async () => {
  const world = makeWorld();
  const badBody = `Broken block.\n\n${METADATA_START}\n\`\`\`json\n{broken}\n\`\`\`\n${METADATA_END}\n`;
  world.writer.seedItem({
    repo: REPO, kind: "issue", number: 14, title: "Broken", body: badBody,
    state: "open", labels: [], createdAt: "2026-11-20T10:00:00.000Z",
  });
  world.writer.seedItem({
    repo: REPO, kind: "issue", number: 15, title: "Fine",
    body: bodyWithBlock("Fine.", authoredBlock(REPO)),
    state: "open", labels: [], createdAt: "2026-11-20T10:00:00.000Z",
  });

  const res = await deliver(world, issuePayload(REPO, 14));
  assert.equal(res.status, 400);
  assert.equal(((await res.json()) as { error: string }).error, "invalid_metadata_block");

  const sweepRes = await sweep(world, INGEST_TOKEN, { repos: [REPO] });
  assert.equal(sweepRes.status, 200, await sweepRes.clone().text());
  const doc = (await sweepRes.json()) as { results: CheckResult[]; counts: { errors: number } };
  const bad = doc.results.find((r) => (r as { number: number }).number === 14) as CheckResult;
  const good = doc.results.find((r) => (r as { number: number }).number === 15) as CheckResult;
  assert.equal(bad.outcome, "error");
  assert.equal(bad.error, "invalid_metadata_block");
  assert.equal(good.outcome, "inserted");
  assert.equal(doc.counts.errors, 1);
});

// --- Acceptance 2 + 3: the sweeper.

test("sweeper catches an item the listener missed, with the same check result as the webhook", async () => {
  const world = makeWorld();
  const body = bodyWithBlock("Same item, two repos.", authoredBlock(REPO));
  world.writer.seedItem({
    repo: REPO, kind: "issue", number: 7, title: "Via webhook", body,
    state: "open", labels: [], createdAt: "2026-11-20T10:00:00.000Z",
  });
  const body2 = bodyWithBlock("Same item, two repos.", authoredBlock(REPO2));
  world.writer.seedItem({
    repo: REPO2, kind: "issue", number: 7, title: "Via sweep", body: body2,
    state: "open", labels: [], createdAt: "2026-11-20T10:00:00.000Z",
  });

  const hookRes = await deliver(world, issuePayload(REPO, 7));
  assert.equal(hookRes.status, 200, await hookRes.clone().text());
  const viaHook = (await hookRes.json()) as CheckResult;

  const sweepRes = await sweep(world, INGEST_TOKEN, { repos: [REPO2] });
  assert.equal(sweepRes.status, 200, await sweepRes.clone().text());
  const doc = (await sweepRes.json()) as { results: CheckResult[] };
  assert.equal(doc.results.length, 1);
  const viaSweep = doc.results[0];
  assert.equal(viaSweep.outcome, viaHook.outcome);
  assert.equal(viaSweep.block, viaHook.block);
  assert.equal(viaSweep.binding_id, viaHook.binding_id);
  assert.equal(viaSweep.sufficient, viaHook.sufficient);
  assert.equal(viaSweep.queue?.action, viaHook.queue?.action);
  assert.ok(await world.store.get(REPO2, "issue", 7));
});

test("sweeper completes a quickly-merged PR's Metadata retroactively", async () => {
  const world = makeWorld();
  world.writer.seedItem({
    repo: REPO, kind: "pr", number: 9, title: "Merged before anyone watched",
    body: bodyWithBlock("A fast PR.", authoredBlock(REPO)),
    state: "closed", labels: [], createdAt: "2026-11-21T08:00:00.000Z", merged: true,
  });
  assert.equal(await world.store.get(REPO, "pr", 9), null);
  const res = await sweep(world, INGEST_TOKEN, { repos: [REPO] });
  assert.equal(res.status, 200, await res.clone().text());
  const doc = (await res.json()) as { results: (CheckResult & { number: number })[] };
  const pr = doc.results.find((r) => r.number === 9);
  assert.ok(pr);
  assert.equal(pr.outcome, "inserted");
  assert.equal(pr.merged, true);
  assert.equal(pr.state, "closed");
  assert.ok(await world.store.get(REPO, "pr", 9), "the merged PR's row is completed retroactively");
});

test("sweeper falls back to the configured scope and fails closed with none", async () => {
  const scoped = makeWorld({ INGESTION_REPOS: REPO });
  scoped.writer.seedItem({
    repo: REPO, kind: "issue", number: 21, title: "Configured scope",
    body: bodyWithBlock("Scoped.", authoredBlock(REPO)),
    state: "open", labels: [], createdAt: "2026-11-20T10:00:00.000Z",
  });
  const okRes = await sweep(scoped, INGEST_TOKEN);
  assert.equal(okRes.status, 200, await okRes.clone().text());
  assert.ok(await scoped.store.get(REPO, "issue", 21));

  const unscoped = makeWorld();
  const res = await sweep(unscoped, INGEST_TOKEN);
  assert.equal(res.status, 503);
  assert.equal(((await res.json()) as { error: string }).error, "ingestion_not_configured");
});

// --- The sweep route rides the shared plumbing like every door.

test("sweep route: unauthenticated 401, cross-tenant 403, unscoped service token 403", async () => {
  const world = makeWorld();
  const anon = await dispatch(
    new Request("https://fleet-hosted.test/v1/ingestion/sweep", { method: "POST" }),
    world.deps,
  );
  assert.equal(anon.status, 401);

  const cross = await dispatch(
    bearerReq("/v1/ingestion/sweep", tokenA, {
      method: "POST",
      headers: { "x-binding-id": BINDING_B, "content-type": "application/json" },
      body: JSON.stringify({ repos: [REPO] }),
    }),
    world.deps,
  );
  assert.equal(cross.status, 403);
  assert.equal(((await cross.json()) as { error: string }).error, "cross_tenant_refused");

  const unscoped = await sweep(world, QUERY_ONLY_TOKEN, { repos: [REPO] });
  assert.equal(unscoped.status, 403);
  assert.equal(((await unscoped.json()) as { error: string }).error, "service_token_route_scope");
});
