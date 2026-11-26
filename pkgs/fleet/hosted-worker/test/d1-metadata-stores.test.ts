// D1 metadata and backfill run store tests (#15456, a slice of the
// #14866 Cloudflare-first programme). Oracles: the fixtures' storage
// contracts (src/stamping.ts FixtureMetadataStore, src/backfill.ts
// FixtureBackfillRunStore) — each D1 store must answer exactly as its
// fixture does for the same calls — plus persistence across store
// instances (a recycled isolate reads back what an earlier one
// wrote), fail-closed decoding of rows the store did not write, and
// the wiring: buildDeps resolves both D1 stores when QUEUE_D1 is
// bound and the fixtures otherwise. The binding is the in-process
// D1 double over a real SQLite file (test/d1-double.ts).

import assert from "node:assert/strict";
import { test } from "node:test";
import { FixtureBackfillRunStore, type BackfillRunRecord, type BackfillRunStore } from "../src/backfill.ts";
import type { HostedEnv } from "../src/config.ts";
import {
  D1BackfillRunStore,
  D1MetadataStore,
  openD1BackfillRunStore,
  openD1MetadataStore,
  readBackfillRunRow,
  readMetadataRow,
} from "../src/d1-metadata-stores.ts";
import { FixtureMetadataStore, extractKeys, type MetadataRecord, type MetadataStore } from "../src/stamping.ts";
import { buildDeps } from "../src/worker.ts";
import { withDb } from "./d1-double.ts";
import { BINDING_A, BINDING_B } from "./helpers.ts";

function metadataRecord(overrides: Partial<MetadataRecord> & { lane?: string; session?: string } = {}): MetadataRecord {
  const { lane = "code-a", session = "ses_abc", ...rest } = overrides;
  const metadata = {
    pm_meta: 1,
    direct_parent: { repo: "developing-today/code", number: 14866 },
    links: [{ target: { repo: "developing-today/code", number: 9081 }, relation: "refs", status: "open" }],
    lineage: { design: null, roadmap_item: null, priority: null },
    provenance: { session_id: session, lane, host: "htch", queue_id: null },
  };
  return {
    repo: "developing-today/code",
    kind: "issue",
    number: 15456,
    url: "https://github.com/developing-today/code/issues/15456",
    bindingId: BINDING_A,
    metadata,
    extracted: extractKeys(metadata),
    insertedAt: "2026-11-26T08:00:00.000Z",
    ...rest,
  };
}

async function exerciseMetadata(store: MetadataStore): Promise<unknown[]> {
  const out: unknown[] = [];
  out.push(await store.get("developing-today/code", "issue", 15456));
  await store.insert(metadataRecord());
  await store.insert(metadataRecord({ number: 15457, url: "u2", lane: "code-b" }));
  await store.insert(metadataRecord({ kind: "pr", number: 15456, url: "u3" }));
  await store.insert(metadataRecord({ number: 15458, url: "u4", bindingId: BINDING_B }));
  await store.insert(metadataRecord({ repo: "developing-today/other", number: 1, url: "u5" }));
  // A re-insert of the same item replaces its row in place.
  await store.insert(metadataRecord({ insertedAt: "2026-11-26T09:00:00.000Z", lane: "code-c" }));
  out.push(await store.get("developing-today/code", "issue", 15456));
  out.push(await store.get("developing-today/code", "pr", 15456));
  out.push(await store.get("developing-today/code", "issue", 1));
  out.push(await store.list(BINDING_A, {}));
  out.push(await store.list(BINDING_A, { lane: "code-b" }));
  out.push(await store.list(BINDING_A, { kind: "pr" }));
  out.push(await store.list(BINDING_A, { repo: "developing-today/other" }));
  out.push(await store.list(BINDING_A, { linkRelation: "refs", linkStatus: "open" }));
  out.push(await store.list(BINDING_A, { directParent: { repo: "developing-today/code", number: 14866 } }));
  out.push(await store.list(BINDING_B, {}));
  out.push(await store.list("binding-none", {}));
  out.push(await store.listRepo("developing-today/code"));
  out.push(await store.listRepo("developing-today/missing"));
  const copy = await store.get("developing-today/code", "issue", 15456);
  copy!.extracted.links.push({ target: null, relation: "mutated", status: null });
  out.push(await store.get("developing-today/code", "issue", 15456));
  return out;
}

test(
  "metadata store contract: the D1 store answers exactly as the fixture does",
  withDb(async (d1) => {
    const expected = await exerciseMetadata(new FixtureMetadataStore());
    const actual = await exerciseMetadata(openD1MetadataStore(d1));
    assert.deepEqual(actual, expected);
    // Sanity on the oracle itself: the scenario reaches real rows.
    assert.equal((expected[12] as MetadataRecord[]).length, 4);
    assert.equal((expected[1] as MetadataRecord).extracted.lane, "code-c");
  }),
);

function run(id: string, overrides: Partial<BackfillRunRecord> = {}): BackfillRunRecord {
  return {
    id,
    scope: ["developing-today/code"],
    startedAt: "2026-11-26T08:00:00.000Z",
    finishedAt: "2026-11-26T08:01:00.000Z",
    counts: { checked: 3, inserted: 1, refreshed: 1, unchanged: 1, errors: 0 },
    failures: [],
    ...overrides,
  };
}

async function exerciseRuns(store: BackfillRunStore): Promise<unknown[]> {
  const out: unknown[] = [];
  out.push(await store.list());
  out.push(await store.get("run-1"));
  await store.record(run("run-1"));
  await store.record(
    run("run-2", {
      counts: { checked: 1, inserted: 0, refreshed: 0, unchanged: 0, errors: 1 },
      failures: [
        {
          repo: "developing-today/code",
          kind: "issue",
          number: 7,
          url: "https://github.com/developing-today/code/issues/7",
          error: "invalid_metadata_block",
          message: "bad block",
        },
      ],
    }),
  );
  // A repeated id: the fixture keeps both and get answers the first.
  await store.record(run("run-1", { finishedAt: "2026-11-26T08:09:00.000Z" }));
  out.push(await store.list());
  out.push(await store.get("run-1"));
  out.push(await store.get("run-2"));
  out.push(await store.get("run-missing"));
  const copy = await store.get("run-2");
  copy!.failures.length = 0;
  out.push(await store.get("run-2"));
  return out;
}

test(
  "backfill run store contract: the D1 store answers exactly as the fixture does",
  withDb(async (d1) => {
    const expected = await exerciseRuns(new FixtureBackfillRunStore());
    const actual = await exerciseRuns(openD1BackfillRunStore(d1));
    assert.deepEqual(actual, expected);
    assert.equal((expected[2] as BackfillRunRecord[]).length, 3);
  }),
);

test(
  "rows outlive the store instance: fresh stores over the same database read them back",
  withDb(async (d1) => {
    await openD1MetadataStore(d1).insert(metadataRecord());
    await openD1BackfillRunStore(d1).record(run("run-1"));
    assert.deepEqual(await openD1MetadataStore(d1).get("developing-today/code", "issue", 15456), metadataRecord());
    assert.deepEqual(await openD1MetadataStore(d1).list(BINDING_A, { sessionId: "ses_abc" }), [metadataRecord()]);
    assert.deepEqual(await openD1BackfillRunStore(d1).list(), [run("run-1")]);
  }),
);

test(
  "rows the stores did not write fail closed instead of answering",
  withDb(async (d1) => {
    const metadata = openD1MetadataStore(d1);
    const runs = openD1BackfillRunStore(d1);
    await metadata.listRepo("warmup");
    await runs.list();
    d1.db
      .prepare("INSERT INTO fleet_metadata (repo, kind, number, binding_id, record_json) VALUES (?, ?, ?, ?, ?)")
      .run("developing-today/code", "issue", 9, BINDING_A, '{"repo":"developing-today/code"}');
    d1.db.prepare("INSERT INTO fleet_backfill_runs (id, record_json) VALUES (?, ?)").run("run-x", "not json");
    await assert.rejects(
      () => metadata.get("developing-today/code", "issue", 9),
      /record_json is not a metadata record/,
    );
    await assert.rejects(() => metadata.list(BINDING_A, {}), /record_json is not a metadata record/);
    await assert.rejects(() => runs.get("run-x"), /record_json is not JSON/);
    assert.throws(() => readMetadataRow({ record_json: null }), /record_json is null, expected text/);
    assert.throws(() => readBackfillRunRow({ record_json: '{"id":"r"}' }), /not a backfill run record/);
  }),
);

test(
  "wiring: buildDeps resolves the D1 stores when QUEUE_D1 is bound, the fixtures otherwise",
  withDb(async (d1) => {
    const bound = buildDeps({ QUEUE_D1: d1 } as HostedEnv);
    assert.ok(bound.metadataStore instanceof D1MetadataStore);
    assert.ok(bound.backfillRunStore instanceof D1BackfillRunStore);
    const unbound = buildDeps({} as HostedEnv);
    assert.ok(unbound.metadataStore instanceof FixtureMetadataStore);
    assert.ok(unbound.backfillRunStore instanceof FixtureBackfillRunStore);
  }),
);
