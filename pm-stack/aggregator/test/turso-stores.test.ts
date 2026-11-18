import { test, expect } from "bun:test";
import { Database } from "bun:sqlite";
import { storeStatement, decodeStoreRow, createTursoStoreClient } from "../src/turso-stores.ts";
import { createSupabaseActivityRemote, createSupabaseDocumentRemote, createSupabaseSnapshotsRemote } from "../src/remote-stores.ts";
import { AlertsStore } from "../src/alerts.ts";

// SQLite is the execution oracle, not a string-only SQL snapshot.
function localClient(db: Database) {
  return { ref: "local-test", async query<T>(input: string): Promise<T[]> {
    const { sql, args } = storeStatement(input);
    return db.query(sql).all(...args.map(a => a.value)).map(r => decodeStoreRow(r as Record<string, unknown>)) as T[];
  }};
}
test("literal text is never changed by dialect conversion", async () => {
  const db = new Database(":memory:");
  const client = localClient(db);
  const literal = "now() public.agg_actions alerts.incidents ::jsonb 'quoted' ?99";
  const rows = await client.query<{text:string}>(`select '${literal.replaceAll("'", "''")}' as text`);
  expect(rows[0].text).toBe(literal);
  db.close();
});
test("activity arrays and document JSON round trip", async () => {
  const db = new Database(":memory:");
  db.exec(`create table agg_activity_ingestion(source text primary key, checkpoint text, seen_ids text, source_status text, updated_at text);
    create table agg_activity_gaps(id text primary key, source text, gap_from text, gap_to text, status text, detected_at text, resolved_at text, updated_at text);
    create table agg_store_documents(store text primary key, version integer, document text, writer text, updated_at text);`);
  const c = localClient(db), get = async () => c;
  const activity = createSupabaseActivityRemote(get);
  const doc = createSupabaseDocumentRemote(get);
  const value = { version: 1 as const, checkpoints: {}, seen: { one: ["a'b", "now()", "x,y"], empty: [] }, gaps: [], sourceStatus: {} };
  await activity.save(value);
  expect((await activity.load())!.seen).toEqual(value.seen);
  await doc.save("test", 1, { quotes: "::jsonb now()", a: [1, true] });
  expect(await doc.load("test")).toEqual({version:1, document:{quotes:"::jsonb now()",a:[1,true]}});
  db.close();
});
test("alerts partial-index dedupe, JSON preference merge and ack are preserved", async () => {
  const db = new Database(":memory:");
  db.exec(`create table alerts_incidents(id text primary key default (lower(hex(randomblob(16)))), source text, incident_key text, severity text, title text, body text, status text default 'open', occurrences integer default 1, first_seen_at text default (strftime('%Y-%m-%dT%H:%M:%fZ','now')), last_seen_at text, delivered_at text, escalated_at text, acked_at text, closed_at text);
    create unique index alert_open_key on alerts_incidents(source,incident_key) where status <> 'closed';
    create table alerts_incident_events(id integer primary key, incident_id text, kind text, detail text);
    create table alerts_notification_sends(id integer primary key, class text, object_ref text, incident_id text, outcome text, acted_on_at text);
    create table alerts_push_subscriptions(id text default 'sub', endpoint text unique, p256dh text, auth text, classes text, quiet text, user_agent text, last_seen_at text, created_at text, gone_count integer default 0);`);
  const store = new AlertsStore(localClient(db));
  const a = await store.postIncident({source:"test", key:"same", severity:"critical", title:"now() ::jsonb"});
  expect(a.created).toBe(true);
  const b = await store.postIncident({source:"test", key:"same", severity:"critical", title:"again"});
  expect(b.created).toBe(false); expect(b.incident.occurrences).toBe(2);
  expect((await store.ackIncident(a.incident.id)).transitioned).toBe(true);
  expect((await store.ackIncident(a.incident.id)).transitioned).toBe(false);
  expect(await store.pendingEscalations(15)).toEqual([]);
  const sub = {endpoint:"https://example.test",p256dh:"test",auth:"test",classes:{critical:true},quiet:{} } as any;
  await store.upsertSubscription(sub);
  const updated = await store.upsertSubscription({...sub,classes:{digest:false},providedClasses:{digest:false}});
  expect(updated.classes).toEqual({critical:true,digest:false});
  db.close();
});
test("transport fails closed on HTTP and malformed envelopes", async () => {
  const c = createTursoStoreClient("https://example.test", "test", (async () => new Response('{}')) as any);
  await expect(c.query("select 1")).rejects.toThrow("malformed");
});
test("snapshots preserve numeric and range JSON values and replay dedupes", async () => {
 const db = new Database(":memory:");
 db.exec(`create table agg_usage_snapshots(snapshot_key text primary key,ts text,service_id text,metric_id text,unit text,source text,value text,limit_value real,resets_at text,observed_by text)`);
 const c=localClient(db), store=createSupabaseSnapshotsRemote(async()=>c);
 const base={ts:"2026-11-18T00:00:00.000Z",serviceId:"service",metricId:"m",value:42,unit:"usd",source:"api",observedBy:"test"} as any;
 expect(await store.append([base])).toBe(1); expect(await store.append([base])).toBe(0);
 expect((await store.readAll())[0].value).toBe(42);
 await store.append([{...base,value:{low:1,high:3}}]);
 expect((await store.readAll())[1].value).toEqual({low:1,high:3}); db.close();
});
test("unknown dialect and unsafe numeric result are rejected", async () => {
 expect(()=>storeStatement("select x::uuid")).toThrow("unsupported");
 const c=createTursoStoreClient("https://example.test","test",(async()=>Response.json({results:[{type:"ok",response:{result:{cols:[{name:"id"}],rows:[[{type:"integer",value:"9007199254740992"}]]}}}]})) as any);
 await expect(c.query("select 1")).rejects.toThrow("unsafe");
});
