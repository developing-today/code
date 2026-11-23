import { test, expect } from "bun:test";
import { Database } from "bun:sqlite";
import {
  storeStatement,
  decodeStoreRow,
  createTursoStoreClient,
} from "../src/turso-stores.ts";
import {
  createTursoActionsRemote,
  createTursoActivityRemote,
  createTursoCommentsRemote,
  createTursoDocumentRemote,
  createTursoSnapshotsRemote,
} from "../src/remote-stores.ts";
import { AlertsStore } from "../src/alerts.ts";
import type { ActivitySourceStatus, IngestionFile } from "../src/activity.ts";

// SQLite is the execution oracle, not a string-only SQL snapshot.
function localClient(db: Database) {
  return {
    ref: "local-test",
    async query<T>(input: string): Promise<T[]> {
      const { sql, args } = storeStatement(input);
      return db
        .query(sql)
        .all(...args.map((a) => a.value))
        .map((r) => decodeStoreRow(r as Record<string, unknown>)) as T[];
    },
  };
}
test("literal text is never changed by dialect conversion", async () => {
  const db = new Database(":memory:");
  const client = localClient(db);
  const literal =
    "now() public.agg_actions alerts.incidents ::jsonb 'quoted' ?99";
  const rows = await client.query<{ text: string }>(
    `select '${literal.replaceAll("'", "''")}' as text`,
  );
  expect(rows[0].text).toBe(literal);
  db.close();
});
test("activity arrays and document JSON round trip", async () => {
  const db = new Database(":memory:");
  db.exec(`create table agg_activity_ingestion(source text primary key, checkpoint text, seen_ids text, source_status text, updated_at text);
    create table agg_activity_gaps(id text primary key, source text, gap_from text, gap_to text, status text, detected_at text, resolved_at text, updated_at text);
    create table agg_store_documents(store text primary key, version integer, document text, writer text, updated_at text);`);
  const c = localClient(db),
    get = async () => c;
  const activity = createTursoActivityRemote(get);
  const doc = createTursoDocumentRemote(get);
  const value = {
    version: 1 as const,
    checkpoints: {},
    seen: { one: ["a'b", "now()", "x,y"], empty: [] },
    gaps: [],
    sourceStatus: {},
  };
  await activity.save(value);
  expect((await activity.load())!.seen).toEqual(value.seen);
  await doc.save("test", 1, { quotes: "::jsonb now()", a: [1, true] });
  expect(await doc.load("test")).toEqual({
    version: 1,
    document: { quotes: "::jsonb now()", a: [1, true] },
  });
  const event = {
    eventId: "report:stable",
    type: "report.generated",
    ts: "2026-11-22T00:00:00.000Z",
  };
  expect(
    await doc.insertIfAbsent("c7-report:v1:generated:stable", 1, event),
  ).toBe(true);
  expect(
    await doc.insertIfAbsent("c7-report:v1:generated:stable", 1, {
      ...event,
      ts: "2026-11-22T00:01:00.000Z",
    }),
  ).toBe(false);
  expect(await doc.listPrefix("c7-report:v1:")).toEqual([
    { store: "c7-report:v1:generated:stable", version: 1, document: event },
  ]);
  db.close();
});
// SQLite's total_changes is the row-mutation oracle, not a billing estimate.
function totalChanges(db: Database): number {
  return db.query<{ n: number }, []>("select total_changes() n").get()!.n;
}

test("activity writes only changed rows and preserves watchdog windows and semantic freshness", async () => {
  using db = new Database(":memory:");
  db.exec(`create table agg_activity_ingestion(source text primary key, checkpoint text, seen_ids text, source_status text, updated_at text);
    create table agg_activity_gaps(id text primary key, source text, gap_from text, gap_to text, status text, detected_at text, resolved_at text, updated_at text);`);
  const t0 = "2026-11-20T01:00:00.000Z",
    t1 = "2026-11-20T01:05:00.000Z";
  const old = "2026-11-18T00:00:00.000Z";
  const status: ActivitySourceStatus = {
    source: "github",
    state: "current",
    consumer: "pm-aggregator",
    cursor: 5,
    checkpointCursor: 5,
    highWater: 5,
    lastAttemptAt: t0,
    lastSuccessAt: t0,
    updatedAt: t0,
  };
  const draft: IngestionFile = {
    version: 1,
    checkpoints: {},
    seen: { github: ["delivery-5"], other: [] },
    sourceStatus: { github: status },
    gaps: [
      {
        id: "open",
        source: "watchdog",
        from: t0,
        to: t0,
        status: "open",
        detectedAt: t0,
      },
      {
        id: "closed",
        source: "github",
        from: t0,
        to: t0,
        status: "backfilled",
        detectedAt: t0,
        resolvedAt: t0,
      },
    ],
  };
  const remote = () => createTursoActivityRemote(async () => localClient(db));
  const writer = remote();
  await writer.save(draft);
  expect(totalChanges(db)).toBe(4);

  db.run("update agg_activity_ingestion set updated_at=?", [old]);
  db.run("update agg_activity_gaps set updated_at=?", [old]);
  let before = totalChanges(db);
  await writer.save(structuredClone(draft));
  expect(totalChanges(db) - before).toBe(0); // was 4 unconditional rewrites
  expect((await writer.load())!.sourceStatus!.github).toEqual(status);
  expect(
    db.query("select updated_at from agg_activity_gaps where id='open'").get(),
  ).toEqual({ updated_at: old });
  expect(
    db
      .query(
        "select updated_at from agg_activity_ingestion where source='github'",
      )
      .get(),
  ).toEqual({
    updated_at: old,
  });

  const extended = structuredClone(draft);
  extended.gaps[0].to = t1;
  before = totalChanges(db);
  await writer.save(extended);
  expect(totalChanges(db) - before).toBe(1); // was 4; only active gap extends
  expect((await writer.load())!.gaps.find((g) => g.id === "open")!.to).toBe(t1);
  expect(
    db.query("select updated_at from agg_activity_gaps where id='open'").get(),
  ).not.toEqual({ updated_at: old });
  expect(
    db
      .query("select updated_at from agg_activity_gaps where id='closed'")
      .get(),
  ).toEqual({ updated_at: old });

  const newer = structuredClone(extended);
  newer.sourceStatus!.github = {
    ...status,
    lastAttemptAt: t1,
    lastSuccessAt: t1,
    updatedAt: t1,
  };
  // These write-once columns are outside the existing conflict SET.
  newer.gaps[0].from = old;
  before = totalChanges(db);
  await writer.save(newer);
  expect(totalChanges(db) - before).toBe(1); // meaningful freshness even though cursor did not change
  expect((await writer.load())!.sourceStatus!.github).toEqual(
    newer.sourceStatus!.github,
  );
  expect((await writer.load())!.gaps.find((g) => g.id === "open")!.from).toBe(
    t0,
  );

  const restart = remote();
  const afterRestart = (await restart.load())!;
  before = totalChanges(db);
  await restart.save({ ...afterRestart, gaps: [] }); // omission has never been a delete
  expect(totalChanges(db) - before).toBe(0);
  expect((await restart.load())!.gaps).toHaveLength(2);

  const resolved = structuredClone(afterRestart);
  const gap = resolved.gaps.find((g) => g.id === "open")!;
  gap.status = "backfilled";
  gap.resolvedAt = t1;
  before = totalChanges(db);
  await Promise.all([restart.save(resolved), writer.save(resolved)]);
  expect(totalChanges(db) - before).toBe(1); // resolvedAt NULL→value, concurrent identical replay no-op
  expect((await writer.load())!.gaps.find((g) => g.id === "open")).toEqual(gap);

  const cleared = structuredClone(resolved);
  const clearedGap = cleared.gaps.find((g) => g.id === "open")!;
  clearedGap.status = "open";
  delete clearedGap.resolvedAt;
  before = totalChanges(db);
  await restart.save(cleared);
  expect(totalChanges(db) - before).toBe(1); // value→NULL not mistaken for equality
  expect((await writer.load())!.gaps.find((g) => g.id === "open")).toEqual(
    clearedGap,
  );
});

test("alerts partial-index dedupe, JSON preference merge and ack are preserved", async () => {
  const db = new Database(":memory:");
  db.exec(`create table alerts_incidents(id text primary key default (lower(hex(randomblob(16)))), source text, incident_key text, severity text, title text, body text, status text default 'open', occurrences integer default 1, first_seen_at text default (strftime('%Y-%m-%dT%H:%M:%fZ','now')), last_seen_at text, delivered_at text, escalated_at text, acked_at text, closed_at text);
    create unique index alert_open_key on alerts_incidents(source,incident_key) where status <> 'closed';
    create table alerts_incident_events(id integer primary key, incident_id text, kind text, detail text);
    create table alerts_notification_sends(id integer primary key, class text, object_ref text, incident_id text, outcome text, acted_on_at text);
    create table alerts_push_subscriptions(id text default 'sub', endpoint text unique, p256dh text, auth text, classes text, quiet text, user_agent text, last_seen_at text, created_at text, gone_count integer default 0);`);
  const store = new AlertsStore(localClient(db));
  const a = await store.postIncident({
    source: "test",
    key: "same",
    severity: "critical",
    title: "now() ::jsonb",
  });
  expect(a.created).toBe(true);
  const b = await store.postIncident({
    source: "test",
    key: "same",
    severity: "critical",
    title: "again",
  });
  expect(b.created).toBe(false);
  expect(b.incident.occurrences).toBe(2);
  expect((await store.ackIncident(a.incident.id)).transitioned).toBe(true);
  expect((await store.ackIncident(a.incident.id)).transitioned).toBe(false);
  expect(await store.pendingEscalations(15)).toEqual([]);
  const c = await store.postIncident({
    source: "test",
    key: "second",
    severity: "critical",
    title: "second",
  });
  expect((await store.pendingDeliveries()).map((x) => x.id)).toContain(
    c.incident.id,
  );
  expect(await store.markDelivered(c.incident.id, {})).toBe(true);
  expect(await store.markDelivered(c.incident.id, {})).toBe(false);
  db.query(
    "update alerts_incidents set delivered_at = strftime('%Y-%m-%dT%H:%M:%fZ','now','-16 minutes') where id = ?",
  ).run(c.incident.id);
  expect((await store.pendingEscalations(15)).map((x) => x.id)).toContain(
    c.incident.id,
  );
  expect(await store.markEscalated(c.incident.id, {})).toBe(true);
  expect(await store.pendingEscalations(15)).toEqual([]);
  await store.logSend({
    class: "critical",
    objectRef: "test",
    incidentId: c.incident.id,
    outcome: "sent",
  });
  expect((await store.listSends()).length).toBe(1);
  await store.ackIncident(c.incident.id);
  expect((await store.listSends())[0].acted_on_at).not.toBeNull();
  expect((await store.closeIncident(c.incident.id)).transitioned).toBe(true);
  expect((await store.closeIncident(c.incident.id)).transitioned).toBe(false);
  expect((await store.recentEvents()).length).toBeGreaterThan(3);
  const sub = {
    endpoint: "https://example.test",
    p256dh: "test",
    auth: "test",
    classes: { critical: true },
    quiet: {},
  } as any;
  await store.upsertSubscription(sub);
  const updated = await store.upsertSubscription({
    ...sub,
    classes: { digest: false },
    providedClasses: { digest: false },
  });
  expect(updated.classes).toEqual({ critical: true, digest: false });
  db.close();
});
test("transport fails closed on HTTP and malformed envelopes", async () => {
  const c = createTursoStoreClient(
    "https://example.test",
    "test",
    (async () => new Response("{}")) as any,
  );
  await expect(c.query("select 1")).rejects.toThrow("malformed");
});
test("snapshots preserve numeric and range JSON values and replay dedupes", async () => {
  const db = new Database(":memory:");
  db.exec(
    `create table agg_usage_snapshots(snapshot_key text primary key,ts text,service_id text,metric_id text,unit text,source text,value text,limit_value real,resets_at text,observed_by text)`,
  );
  const c = localClient(db),
    store = createTursoSnapshotsRemote(async () => c);
  const base = {
    ts: "2026-11-18T00:00:00.000Z",
    serviceId: "service",
    metricId: "m",
    value: 42,
    unit: "usd",
    source: "api",
    observedBy: "test",
  } as any;
  expect(await store.append([base])).toBe(1);
  expect(await store.append([base])).toBe(0);
  expect((await store.readAll())[0].value).toBe(42);
  await store.append([{ ...base, value: { low: 1, high: 3 } }]);
  expect((await store.readAll())[1].value).toEqual({ low: 1, high: 3 });
  db.close();
});
test("unknown dialect and unsafe numeric result are rejected", async () => {
  expect(() => storeStatement("select x::uuid")).toThrow("unsupported");
  const c = createTursoStoreClient("https://example.test", "test", (async () =>
    Response.json({
      results: [
        {
          type: "ok",
          response: {
            result: {
              cols: [{ name: "id" }],
              rows: [[{ type: "integer", value: "9007199254740992" }]],
            },
          },
        },
      ],
    })) as any);
  await expect(c.query("select 1")).rejects.toThrow("unsafe");
});

test("actions JSON and self references round trip with real FK constraints", async () => {
  const db = new Database(":memory:");
  db.exec(`pragma foreign_keys=on;
 create table agg_actions(id text primary key,client_token text,kind text,verb text,object_ref text,target_ref text,session_id text,session_source text,payload text,author text,created_at text,state text,applied_at text,applied_by_run text,result_note text,supersedes_action_id text references agg_actions(id),duplicate_of_action_id text references agg_actions(id));`);
  const store = createTursoActionsRemote(async () => localClient(db));
  const base = {
    id: "one",
    kind: "task",
    verb: "park",
    objectRef: "repo#1",
    payload: { text: "now() 'x'" },
    author: { kind: "drew", id: "drew", label: "Drew" },
    createdAt: "2026-11-18T00:00:00.000Z",
    state: "new",
  } as any;
  await store.saveAll([
    {
      ...base,
      id: "two",
      supersedes: "one",
      createdAt: "2026-11-18T00:01:00.000Z",
    },
    base,
  ]);
  const rows = await store.loadAll();
  expect(rows[0].payload).toEqual(base.payload);
  expect(rows[0].author).toEqual(base.author);
  expect(rows[1].supersedes).toBe("one");
  let before = totalChanges(db);
  await store.saveAll(rows);
  expect(totalChanges(db) - before).toBe(0); // was 2 unconditional rewrites

  const changed = { ...rows[0], resultNote: "new result" };
  const invalid = { ...rows[1], supersedes: "no-such-parent" };
  await expect(store.saveAll([changed, invalid])).rejects.toThrow(
    "FOREIGN KEY",
  );
  expect(await store.loadAll()).toEqual(rows); // same statement remains atomic
  before = totalChanges(db);
  await store.saveAll([changed, rows[1]]);
  expect(totalChanges(db) - before).toBe(1);
  expect((await store.loadAll())[0].resultNote).toBe("new result");
  before = totalChanges(db);
  await store.saveAll(rows); // genuinely different nullable fields still cleared by the next writer
  expect(totalChanges(db) - before).toBe(1);
  expect(await store.loadAll()).toEqual(rows);
  db.close();
});
test("shared_comments round-trips through the dialect bridge (SQLite oracle)", async () => {
  const db = new Database(":memory:");
  db.exec(
    `create table shared_comments(id text primary key, object_ref text, kind text, author text, author_verified integer, provenance text, linked_identity text, body text, created_at text, triage_state text, github_ref text, github_state text, github_url text, github_error text, github_failed_at text);`,
  );
  const remote = createTursoCommentsRemote(async () => localClient(db));
  const c = {
    id: "dc-bridge-1",
    objectRef: "orch:item-9",
    kind: "message",
    author: { kind: "drew", id: "drew", label: "Drew" },
    authorVerified: true,
    provenance: "verified",
    body: "now() public.shared_comments ::jsonb 'quoted'",
    createdAt: "2026-11-18T00:00:00.000Z",
    triageState: "new",
  } as any;
  const unrelated = {
    ...c,
    id: "other",
    createdAt: "2026-11-18T00:01:00.000Z",
  };
  await remote.saveAll([unrelated, c]);
  expect(await remote.loadAll()).toEqual([c, unrelated]);
  let before = totalChanges(db);
  await remote.saveAll([unrelated, { ...c, body: "immutable" }]);
  expect(totalChanges(db) - before).toBe(0); // was 2 writes, even though no writable field changed
  const triaged = {
    ...c,
    triageState: "triaged",
    githubState: "failed",
    githubError: "test",
  };
  await remote.saveAll([unrelated, triaged]);
  expect(totalChanges(db) - before).toBe(1);
  let rows = await remote.loadAll();
  expect(rows).toEqual([triaged, unrelated]);
  before = totalChanges(db);
  await remote.saveAll([{ ...triaged, githubError: undefined }]);
  expect(totalChanges(db) - before).toBe(1); // nullable edit lands; absent row left alone
  rows = await remote.loadAll();
  expect(rows).toHaveLength(2);
  expect(rows[0].githubError).toBeUndefined();
  db.close();
});
