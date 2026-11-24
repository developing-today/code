// Backfill + divergence tests (#14312). Oracles: the issue's
// acceptance — (1) repository backfill populates the metadata DB
// idempotently, with a recorded backfill run naming counts in/out
// and any failures; (2) re-running the backfill is a no-op (the
// second run's record shows 0 inserted / 0 refreshed); (3) a
// body-vs-DB divergence report names the divergence classes
// (missing row, stale field, orphan row) with counts from a real
// run — plus the doors' idiom: the new routes ride the shared
// plumbing (401 unauthenticated, 403 for a service token scoped
// to another door) and fail closed 503 on their unconfigured
// seams, and a per-item failure is named in the run record while
// the run continues (the sweep's rule, inherited by the backfill).

import assert from "node:assert/strict";
import { test } from "node:test";
import { sha256Hex, type AuthDeps } from "../src/auth.ts";
import { FixtureBackfillRunStore, type BackfillRunRecord } from "../src/backfill.ts";
import { configFromEnv } from "../src/config.ts";
import { FixtureGitHubWriter } from "../src/github.ts";
import { METADATA_END, METADATA_START, parseMetadataBlock, renderBodyWithBlock } from "../src/metadata.ts";
import { FixtureQueueStore } from "../src/queue-store.ts";
import { dispatch } from "../src/router.ts";
import { FixtureSessionRegistry } from "../src/sessions.ts";
import { extractKeys, FixtureMetadataStore, FixtureQueueRecordSource } from "../src/stamping.ts";
import { AUDIENCE, BINDING_A, ISSUER, bearerReq, makeHarness } from "./helpers.ts";

const h = await makeHarness();

const REPO = "developing-today/code";
const FIXED_MS = Date.parse("2026-11-24T12:00:00.000Z");
const FIXED_ISO = "2026-11-24T12:00:00.000Z";
const INGEST_TOKEN = "svc-token-ingestion-0123456789abcdef";
const QUERY_ONLY_TOKEN = "svc-token-query-only-0123456789abcde";

const tenancyFixture = JSON.stringify({
  subjects: [{ issuer: ISSUER, subject: "user-sub-1", binding_id: BINDING_A, active: true }],
  service_tokens: [
    {
      sha256: await sha256Hex(INGEST_TOKEN),
      binding_id: BINDING_A,
      routes: ["ingestion"],
      active: true,
      label: "ingestion-test",
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

interface World {
  deps: AuthDeps;
  store: FixtureMetadataStore;
  writer: FixtureGitHubWriter;
  runs: FixtureBackfillRunStore;
}

function makeWorld(): World {
  const store = new FixtureMetadataStore();
  const writer = new FixtureGitHubWriter();
  const runs = new FixtureBackfillRunStore();
  const config = configFromEnv({
    RP_ISSUER: ISSUER,
    RP_AUDIENCE: AUDIENCE,
    TENANCY_FIXTURE_JSON: tenancyFixture,
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
      queueStore: new FixtureQueueStore(),
      backfillRunStore: runs,
      now: () => FIXED_MS,
    },
    store,
    writer,
    runs,
  };
}

/** A §12 block as a native (other-edition) writer would have
 * stamped it (the ingestion tests' authoredBlock idiom). */
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

function seedAuthored(world: World, number: number): void {
  world.writer.seedItem({
    repo: REPO,
    kind: "issue",
    number,
    title: `Item ${number}`,
    body: bodyWithBlock(`Body of ${number}.`, authoredBlock(REPO)),
    state: "open",
    labels: [],
    createdAt: "2026-11-20T10:00:00.000Z",
  });
}

async function backfill(world: World, token: string = INGEST_TOKEN, body?: unknown): Promise<Response> {
  return dispatch(
    bearerReq("/v1/ingestion/backfill", token, {
      method: "POST",
      headers: { "content-type": "application/json" },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    }),
    world.deps,
  );
}

async function divergence(world: World, token: string = INGEST_TOKEN, body?: unknown): Promise<Response> {
  return dispatch(
    bearerReq("/v1/ingestion/divergence", token, {
      method: "POST",
      headers: { "content-type": "application/json" },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    }),
    world.deps,
  );
}

// --- Acceptance 1: the backfill populates and records the run.

test("backfill populates the store and records a run that reads back", async () => {
  const world = makeWorld();
  seedAuthored(world, 7);
  world.writer.seedItem({
    repo: REPO,
    kind: "issue",
    number: 8,
    title: "No block yet",
    body: "Just prose.",
    state: "open",
    labels: [],
    createdAt: "2026-11-21T09:00:00.000Z",
  });

  const res = await backfill(world, INGEST_TOKEN, { repos: [REPO] });
  assert.equal(res.status, 201, await res.clone().text());
  const run = (await res.json()) as BackfillRunRecord;
  assert.deepEqual(run.scope, [REPO]);
  assert.equal(run.startedAt, FIXED_ISO);
  assert.equal(run.finishedAt, FIXED_ISO);
  assert.deepEqual(run.counts, { checked: 2, inserted: 2, refreshed: 0, unchanged: 0, errors: 0 });
  assert.deepEqual(run.failures, []);

  assert.ok(await world.store.get(REPO, "issue", 7), "the authored item is stored");
  assert.ok(await world.store.get(REPO, "issue", 8), "the block-less item is stored (ascertained)");

  // The record reads back both ways: the run store by id, and the
  // runs route as the operator reads it.
  const stored = await world.runs.get(run.id);
  assert.deepEqual(stored, run);
  const listRes = await dispatch(bearerReq("/v1/ingestion/backfill/runs", INGEST_TOKEN), world.deps);
  assert.equal(listRes.status, 200, await listRes.clone().text());
  const doc = (await listRes.json()) as { runs: BackfillRunRecord[] };
  assert.equal(doc.runs.length, 1);
  assert.equal(doc.runs[0].id, run.id);
});

// --- Acceptance 2: re-running is a no-op, and the record says so.

test("re-running a backfill over an unchanged store records 0 inserted / 0 refreshed", async () => {
  const world = makeWorld();
  seedAuthored(world, 7);
  world.writer.seedItem({
    repo: REPO,
    kind: "issue",
    number: 8,
    title: "No block yet",
    body: "Just prose.",
    state: "open",
    labels: [],
    createdAt: "2026-11-21T09:00:00.000Z",
  });

  const first = (await (await backfill(world, INGEST_TOKEN, { repos: [REPO] })).json()) as BackfillRunRecord;
  assert.deepEqual(first.counts, { checked: 2, inserted: 2, refreshed: 0, unchanged: 0, errors: 0 });

  const secondRes = await backfill(world, INGEST_TOKEN, { repos: [REPO] });
  assert.equal(secondRes.status, 201, await secondRes.clone().text());
  const second = (await secondRes.json()) as BackfillRunRecord;
  assert.notEqual(second.id, first.id);
  assert.deepEqual(second.counts, { checked: 2, inserted: 0, refreshed: 0, unchanged: 2, errors: 0 });
  assert.deepEqual(second.failures, []);
  assert.equal((await world.runs.list()).length, 2, "both runs are recorded");
});

// --- Failures are named by item while the run continues.

test("a malformed-block item is named in the run's failures while the run continues", async () => {
  const world = makeWorld();
  const badBody = `Broken block.\n\n${METADATA_START}\n\`\`\`json\n{broken}\n\`\`\`\n${METADATA_END}\n`;
  world.writer.seedItem({
    repo: REPO,
    kind: "issue",
    number: 14,
    title: "Broken",
    body: badBody,
    state: "open",
    labels: [],
    createdAt: "2026-11-20T10:00:00.000Z",
  });
  seedAuthored(world, 15);

  const res = await backfill(world, INGEST_TOKEN, { repos: [REPO] });
  assert.equal(res.status, 201, await res.clone().text());
  const run = (await res.json()) as BackfillRunRecord;
  assert.deepEqual(run.counts, { checked: 2, inserted: 1, refreshed: 0, unchanged: 0, errors: 1 });
  assert.equal(run.failures.length, 1);
  const failure = run.failures[0];
  assert.equal(failure.repo, REPO);
  assert.equal(failure.kind, "issue");
  assert.equal(failure.number, 14);
  assert.equal(failure.url, `https://github.com/${REPO}/issues/14`);
  assert.equal(failure.error, "invalid_metadata_block");
  assert.ok(await world.store.get(REPO, "issue", 15), "the good item still lands");
  assert.equal(await world.store.get(REPO, "issue", 14), null, "the broken item stores nothing");
});

// --- Acceptance 3: the divergence report names all three classes.

interface DivergenceDoc {
  repos: string[];
  counts: { items: number; rows: number; agreeing: number; missing_row: number; stale_field: number; orphan_row: number };
  findings: { class: string; number: number; fields?: string[]; binding_id?: string }[];
}

test("divergence report names missing rows, stale fields and orphan rows with counts", async () => {
  const world = makeWorld();
  seedAuthored(world, 22);
  seedAuthored(world, 23);
  const fill = await backfill(world, INGEST_TOKEN, { repos: [REPO] });
  assert.equal(fill.status, 201, await fill.clone().text());

  // Item 21 arrives after the backfill: no row yet → missing_row.
  seedAuthored(world, 21);
  // Item 22's body is edited after the backfill: its lineage now
  // disagrees with the stored row → stale_field on "lineage".
  await world.writer.updateItemBody(
    REPO,
    "issue",
    22,
    bodyWithBlock("Edited.", authoredBlock(REPO, { lineage: { design: null, roadmap_item: null, priority: "high" } })),
  );
  // A row whose item GitHub no longer has → orphan_row.
  const orphanBlock = authoredBlock(REPO);
  await world.store.insert({
    repo: REPO,
    kind: "issue",
    number: 99,
    url: `https://github.com/${REPO}/issues/99`,
    bindingId: BINDING_A,
    metadata: orphanBlock,
    extracted: extractKeys(orphanBlock),
    insertedAt: FIXED_ISO,
  });

  const res = await divergence(world, INGEST_TOKEN, { repos: [REPO] });
  assert.equal(res.status, 200, await res.clone().text());
  const doc = (await res.json()) as DivergenceDoc;
  assert.deepEqual(doc.repos, [REPO]);
  assert.deepEqual(doc.counts, { items: 3, rows: 3, agreeing: 1, missing_row: 1, stale_field: 1, orphan_row: 1 });
  const byClass = new Map(doc.findings.map((f) => [f.class, f]));
  assert.equal(byClass.get("missing_row")?.number, 21);
  assert.equal(byClass.get("stale_field")?.number, 22);
  assert.deepEqual(byClass.get("stale_field")?.fields, ["lineage"]);
  assert.equal(byClass.get("orphan_row")?.number, 99);
  assert.equal(byClass.get("orphan_row")?.binding_id, BINDING_A);

  // The report is a read: the stale row still holds the old
  // lineage (reported, never silently overwritten — §15).
  const row22 = await world.store.get(REPO, "issue", 22);
  assert.equal((row22?.metadata["lineage"] as { priority: unknown }).priority, null);
});

test("divergence over an agreeing scope reports zero findings", async () => {
  const world = makeWorld();
  seedAuthored(world, 7);
  const fill = await backfill(world, INGEST_TOKEN, { repos: [REPO] });
  assert.equal(fill.status, 201, await fill.clone().text());
  const res = await divergence(world, INGEST_TOKEN, { repos: [REPO] });
  assert.equal(res.status, 200, await res.clone().text());
  const doc = (await res.json()) as DivergenceDoc;
  assert.deepEqual(doc.counts, { items: 1, rows: 1, agreeing: 1, missing_row: 0, stale_field: 0, orphan_row: 0 });
  assert.deepEqual(doc.findings, []);
});

// --- The doors' idiom: plumbing + fail-closed seams.

test("backfill routes ride the shared plumbing: unauthenticated 401, unscoped service token 403", async () => {
  const world = makeWorld();
  const anon = await dispatch(
    new Request("https://fleet-hosted.test/v1/ingestion/backfill", { method: "POST" }),
    world.deps,
  );
  assert.equal(anon.status, 401);
  const anonRuns = await dispatch(new Request("https://fleet-hosted.test/v1/ingestion/backfill/runs"), world.deps);
  assert.equal(anonRuns.status, 401);

  const unscoped = await backfill(world, QUERY_ONLY_TOKEN, { repos: [REPO] });
  assert.equal(unscoped.status, 403);
  assert.equal(((await unscoped.json()) as { error: string }).error, "service_token_route_scope");
});

test("backfill fails closed 503 without a run store; the runs list and divergence on their own seams", async () => {
  const world = makeWorld();
  const noRuns: AuthDeps = { ...world.deps, backfillRunStore: undefined };
  const res = await dispatch(
    bearerReq("/v1/ingestion/backfill", INGEST_TOKEN, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ repos: [REPO] }),
    }),
    noRuns,
  );
  assert.equal(res.status, 503);
  assert.equal(((await res.json()) as { error: string }).error, "backfill_run_store_not_configured");

  const listRes = await dispatch(bearerReq("/v1/ingestion/backfill/runs", INGEST_TOKEN), noRuns);
  assert.equal(listRes.status, 503);
  assert.equal(((await listRes.json()) as { error: string }).error, "backfill_run_store_not_configured");

  const noStore: AuthDeps = { ...world.deps, metadataStore: undefined };
  const divRes = await dispatch(
    bearerReq("/v1/ingestion/divergence", INGEST_TOKEN, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ repos: [REPO] }),
    }),
    noStore,
  );
  assert.equal(divRes.status, 503);
  assert.equal(((await divRes.json()) as { error: string }).error, "metadata_store_not_configured");
});
