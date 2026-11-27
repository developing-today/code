// Slice 3 of #15538: the D1 bridge store appends and reads an event
// plus its chain head through an explicit per-tenant D1 binding, and
// a second tenant binding cannot read the first tenant's event.
//
// Oracle: the #15538 acceptance criterion — exact event bytes and
// chain head are returned only by the binding used for append. The
// fake below implements the same async prepare/bind/run/all/first
// surface as a D1 binding, backed by two independent in-memory
// SQLite databases, so "the other tenant's database" is a separate
// object by construction, exactly as per-tenant D1 databases are.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { D1BridgeStore, type D1Binding, type D1BoundStatement } from "../src/d1-bridge-store.ts";

function fakeD1Binding(): D1Binding {
  const db = new DatabaseSync(":memory:");
  const prepare = (query: string): D1BoundStatement => {
    let values: unknown[] = [];
    const statement: D1BoundStatement = {
      bind(...bound: Array<string | number | null | Uint8Array | ArrayBuffer>) {
        values = bound;
        return statement;
      },
      async run() {
        db.prepare(query).run(...(values as never[]));
      },
      async all<T extends Record<string, unknown>>() {
        const results = db.prepare(query).all(...(values as never[])) as T[];
        return { results };
      },
      async first<T extends Record<string, unknown>>() {
        const row = db.prepare(query).get(...(values as never[])) as T | undefined;
        return row ?? null;
      },
    };
    return statement;
  };
  return {
    prepare,
    async batch<T>(statements: D1BoundStatement[]) {
      db.exec("BEGIN");
      try {
        for (const statement of statements) await statement.run();
        db.exec("COMMIT");
      } catch (error) {
        db.exec("ROLLBACK");
        throw error;
      }
      return [] as T[];
    },
  };
}

const AUTHOR = "aa".repeat(32);
const EVENT_ID = "bb".repeat(32);
const EVENT_BYTES = new TextEncoder().encode("signed-event-bytes-0");

test("append/read/head round trip through one tenant binding; the other binding reads nothing", async () => {
  const tenantA = new D1BridgeStore(fakeD1Binding());
  const tenantB = new D1BridgeStore(fakeD1Binding());
  await tenantA.ensureSchema();
  await tenantB.ensureSchema();

  const head = await tenantA.appendEvent({ author: AUTHOR, seq: 0, id: EVENT_ID, bytes: EVENT_BYTES });
  assert.deepEqual(head, { next_seq: 1, head: EVENT_ID });

  const stored = await tenantA.storedEvent(AUTHOR, 0);
  assert.ok(stored !== null, "tenant A binding must read the appended event");
  assert.equal(stored.id, EVENT_ID);
  assert.deepEqual(stored.bytes, EVENT_BYTES);

  const headA = await tenantA.chainHead(AUTHOR);
  assert.deepEqual(headA, { next_seq: 1, head: EVENT_ID });

  // The isolation oracle: the second tenant's binding — a separate
  // database — holds no event and no head for this author.
  assert.equal(await tenantB.storedEvent(AUTHOR, 0), null);
  assert.deepEqual(await tenantB.chainHead(AUTHOR), { next_seq: 0, head: null });
});

test("a non-contiguous append is refused, never silently stored", async () => {
  const store = new D1BridgeStore(fakeD1Binding());
  await store.ensureSchema();
  await assert.rejects(
    store.appendEvent({ author: AUTHOR, seq: 1, id: EVENT_ID, bytes: EVENT_BYTES }),
    /sequence_gap/,
  );
  assert.equal(await store.storedEvent(AUTHOR, 1), null);
});
