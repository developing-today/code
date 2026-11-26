// #3404 slice 4: the coordinator drives a migration end to end under a
// lease, resumes from any phase after a crash or a lost response, and
// aborts cleanly before the switch. Stale coordinators are refused by
// the lease (directory) and by routing epochs (user objects).
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { DatabaseSync } from "node:sqlite";
import { UserDO, HDR_BINDING, HDR_JURISDICTION, HDR_ROUTED, blobObjectKey } from "../src/user-do.ts";
import { HDR_MIGRATION } from "../src/migration-fence.ts";
import {
  PersistentHostedAccountDirectory,
  type HostedDirectoryDatabase,
} from "../src/persistent-account-directory.ts";
import { abortMigration, driveMigration, workerMigrationPorts, type MigrationPorts } from "../src/migration-coordinator.ts";
import { FailClosed } from "../src/routing.ts";

const A = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

function fakeSql() {
  const db = new DatabaseSync(":memory:");
  return {
    exec(query: string, ...params: unknown[]) {
      const stmt = db.prepare(query);
      if (/^\s*select/i.test(query)) {
        const rows = stmt.all(...(params as never[])) as Record<string, unknown>[];
        return { toArray: () => rows };
      }
      stmt.run(...(params as never[]));
      return { toArray: () => [] as Record<string, unknown>[] };
    },
  };
}

function fakeBucket() {
  const objects = new Map<string, Uint8Array>();
  return {
    objects,
    async head(key: string) {
      const v = objects.get(key);
      return v ? { size: v.byteLength } : null;
    },
    async get(key: string) {
      const v = objects.get(key);
      return v ? { size: v.byteLength, body: new Blob([v]).stream() } : null;
    },
    async put(key: string, value: Uint8Array) {
      objects.set(key, value);
    },
    async delete(key: string) {
      objects.delete(key);
    },
  };
}

function makeDO() {
  const bucket = fakeBucket();
  const state = {
    id: { name: A },
    storage: { sql: fakeSql() },
    blockConcurrencyWhile: async <T>(fn: () => Promise<T>): Promise<T> => fn(),
    acceptWebSocket: () => {},
    getWebSockets: () => [],
  };
  return { d: new UserDO(state as never, { BLOBS: bucket } as never), bucket };
}

/** Lets a test make the k-th directory transition commit and then
 * lose its response, the ambiguous-CAS case. */
interface Faults {
  loseTransition: number;
}

function world(faults: Faults = { loseTransition: 0 }) {
  const db = new DatabaseSync(":memory:");
  for (const name of ["0001_hosted_accounts.sql", "0002_legacy_hosted_accounts.sql", "0003_migration_leases.sql"]) {
    db.exec(readFileSync(new URL(`../migrations/${name}`, import.meta.url), "utf8"));
  }
  let transitions = 0;
  const adapter: HostedDirectoryDatabase = {
    prepare(query) {
      let values: Array<string | number | null> = [];
      return {
        bind(...bound) {
          values = bound;
          return this;
        },
        async first<T extends Record<string, unknown>>() {
          const row = (db.prepare(query).get(...values) as T | undefined) ?? null;
          if (query.startsWith("UPDATE hosted_accounts") && row !== null && ++transitions === faults.loseTransition) {
            throw new Error("connection reset after commit");
          }
          return row;
        },
        async all<T extends Record<string, unknown>>() {
          return db.prepare(query).all(...values) as T[];
        },
      };
    },
  };
  const directory = new PersistentHostedAccountDirectory(adapter);
  const objects = { us: makeDO(), eu: makeDO() } as Record<string, ReturnType<typeof makeDO>>;
  const users = {
    jurisdiction(j: string) {
      return {
        jurisdiction: users.jurisdiction,
        idFromName: (name: string) => ({ name, j }),
        get: (id: { j: string }) => ({ fetch: (r: Request) => objects[id.j].d.fetch(r) }),
      };
    },
    idFromName: () => {
      throw new Error("unrestricted namespace used");
    },
    get: () => {
      throw new Error("unrestricted namespace used");
    },
  };
  let now = 1_000_000;
  const base = workerMigrationPorts({ USERS: users as never }, directory);
  const ports: MigrationPorts = {
    ...base,
    // Each jurisdiction has its own bucket here, so the copy is real.
    async copyBlob(bindingId, hash, cse, from, to) {
      const key = blobObjectKey(bindingId, hash, cse);
      const bytes = objects[from].bucket.objects.get(key);
      if (!bytes) throw new Error(`missing ${hash}`);
      objects[to].bucket.objects.set(key, bytes);
    },
    now: () => now,
  };
  return {
    db,
    directory,
    objects,
    ports,
    advance(ms: number) {
      now += ms;
    },
  };
}

type World = ReturnType<typeof world>;

function routed(w: World, j: string, path: string, init: RequestInit = {}): Request {
  const headers = new Headers(init.headers);
  headers.set(HDR_BINDING, A);
  headers.set(HDR_JURISDICTION, j);
  headers.set(HDR_ROUTED, "restricted");
  return new Request("https://do" + path, { ...init, headers });
}

async function call(w: World, j: string, path: string, init: RequestInit = {}) {
  const r = await w.objects[j].d.fetch(routed(w, j, path, init));
  return { status: r.status, body: (await r.json()) as Record<string, unknown> };
}

async function seed(w: World) {
  await w.directory.signup({ subjectKey: "subject", bindingId: A, jurisdiction: "us" });
  assert.equal((await call(w, "us", "/v1/kv/note", { method: "PUT", body: "hello" })).status, 200);
  assert.equal(
    (await call(w, "us", "/v1/blobs", { method: "PUT", body: new TextEncoder().encode("blob bytes") })).status,
    200,
  );
}

const RUN = { subjectKey: "subject", migrationId: "m-1", targetJurisdiction: "eu", owner: "coord-a", leaseTtlMs: 60_000 };

async function assertMoved(w: World) {
  const record = await w.directory.get("subject");
  assert.deepEqual(
    [record?.phase, record?.jurisdiction, record?.jurisdictionPending, record?.migrationId, record?.routingEpoch],
    ["stable", "eu", null, null, 2],
  );
  assert.equal((await call(w, "eu", "/v1/kv/note")).body.value, "hello", "the target serves the data");
  assert.equal(w.objects.eu.bucket.objects.size, 1, "the blob bytes were copied");
  const write = await call(w, "us", "/v1/kv/late", { method: "PUT", body: "x" });
  assert.equal(write.body.error, "source_sealed", "the source never takes a write again");
}

test("a migration runs end to end and releases its lease", async () => {
  const w = world();
  await seed(w);
  const done = await driveMigration(w.ports, RUN);
  assert.equal(done.jurisdiction, "eu");
  await assertMoved(w);
  const other = await w.directory.acquireLease("subject", "coord-b", w.ports.now(), 1000);
  assert.equal(other.token, 2, "the released lease passes to the next coordinator");
  w.advance(2000);
  assert.equal((await driveMigration(w.ports, { ...RUN, owner: "coord-c" })).jurisdiction, "eu", "rerunning a finished migration is a no-op");
});

for (let failAt = 1; failAt <= 6; failAt++) {
  test(`a coordinator that crashes at user-object call ${failAt} resumes to completion`, async () => {
    const w = world();
    await seed(w);
    let calls = 0;
    const flaky: MigrationPorts = {
      ...w.ports,
      async control(...args) {
        if (++calls === failAt) throw new Error("coordinator crashed");
        return w.ports.control(...args);
      },
    };
    await assert.rejects(driveMigration(flaky, RUN), /coordinator crashed/);
    await driveMigration(w.ports, RUN);
    await assertMoved(w);
  });
}

for (let lose = 1; lose <= 5; lose++) {
  test(`an ambiguous directory step ${lose} (committed, response lost) is retried safely`, async () => {
    const w = world({ loseTransition: lose });
    await seed(w);
    await assert.rejects(driveMigration(w.ports, RUN), /connection reset/);
    await driveMigration(w.ports, RUN);
    await assertMoved(w);
  });
}

test("a second coordinator waits for the lease; a stale one loses it", async () => {
  const w = world();
  await seed(w);
  const leaseA = await w.directory.acquireLease("subject", "coord-a", w.ports.now(), 10_000);
  const stable = (await w.directory.get("subject"))!;
  const pendingA = await w.directory.transition(
    stable,
    { type: "begin", migrationId: "m-1", targetJurisdiction: "eu" },
    { lease: leaseA, nowMs: w.ports.now() },
  );
  await assert.rejects(
    driveMigration(w.ports, { ...RUN, owner: "coord-b" }),
    (e: unknown) => e instanceof FailClosed && e.code === "lease_held",
  );
  w.advance(20_000);
  await driveMigration(w.ports, { ...RUN, owner: "coord-b" });
  await assertMoved(w);
  await assert.rejects(
    w.directory.transition(
      pendingA,
      { type: "freeze-acknowledged", migrationId: "m-1" },
      { lease: leaseA, nowMs: w.ports.now() },
    ),
    (e: unknown) => e instanceof FailClosed && e.code === "lease_lost",
  );
});

test("a directory step under an expired lease is refused even with a current record", async () => {
  const w = world();
  await seed(w);
  const lease = await w.directory.acquireLease("subject", "coord-a", w.ports.now(), 1000);
  w.advance(5000);
  const stable = (await w.directory.get("subject"))!;
  await assert.rejects(
    w.directory.transition(stable, { type: "begin", migrationId: "m-1", targetJurisdiction: "eu" }, { lease, nowMs: w.ports.now() }),
    (e: unknown) => e instanceof FailClosed && e.code === "lease_lost",
  );
  assert.equal((await w.directory.get("subject"))?.phase, "stable");
});

test("abort before the switch reopens the source and refuses a stale freeze; a new migration then succeeds", async () => {
  const w = world();
  await seed(w);
  let calls = 0;
  const stopAtImport: MigrationPorts = {
    ...w.ports,
    async control(...args) {
      if (++calls === 3) throw new Error("coordinator crashed");
      return w.ports.control(...args);
    },
  };
  await assert.rejects(driveMigration(stopAtImport, RUN), /coordinator crashed/);
  assert.equal((await w.directory.get("subject"))?.phase, "frozen");
  assert.equal((await call(w, "us", "/v1/kv/x", { method: "PUT", body: "1" })).body.error, "source_frozen");

  const aborted = await abortMigration(w.ports, RUN);
  assert.deepEqual([aborted.phase, aborted.jurisdiction, aborted.routingEpoch], ["stable", "us", 2]);
  assert.equal((await call(w, "us", "/v1/kv/x", { method: "PUT", body: "1" })).status, 200, "the source reopens");
  assert.deepEqual(await abortMigration(w.ports, RUN), aborted, "abort is repeatable");

  const staleFreeze = await w.objects.us.d.fetch(
    routed(w, "us", "/v1/migration/freeze", {
      method: "POST",
      headers: { [HDR_MIGRATION]: "1" },
      body: JSON.stringify({ migration_id: "m-1", routing_epoch: 1 }),
    }),
  );
  assert.equal(((await staleFreeze.json()) as { error: string }).error, "stale_epoch");

  await driveMigration(w.ports, { ...RUN, migrationId: "m-2" });
  const record = await w.directory.get("subject");
  assert.deepEqual([record?.jurisdiction, record?.routingEpoch], ["eu", 4]);
  assert.equal((await call(w, "eu", "/v1/kv/x")).body.value, "1", "writes made after the abort moved too");
});

test("abort after the switch is refused; the migration can only finish", async () => {
  const w = world();
  await seed(w);
  let calls = 0;
  const stopAtActivate: MigrationPorts = {
    ...w.ports,
    async control(...args) {
      if (++calls === 5) throw new Error("coordinator crashed");
      return w.ports.control(...args);
    },
  };
  await assert.rejects(driveMigration(stopAtActivate, RUN), /coordinator crashed/);
  assert.equal((await w.directory.get("subject"))?.phase, "switched");
  await assert.rejects(
    abortMigration(w.ports, RUN),
    (e: unknown) => e instanceof FailClosed && e.code === "directory_transition_refused",
  );
  await driveMigration(w.ports, RUN);
  await assertMoved(w);
});

test("a coordinator for another migration does not trample a running one", async () => {
  const w = world();
  await seed(w);
  let calls = 0;
  const stopEarly: MigrationPorts = {
    ...w.ports,
    async control(...args) {
      if (++calls === 1) throw new Error("coordinator crashed");
      return w.ports.control(...args);
    },
  };
  await assert.rejects(driveMigration(stopEarly, RUN), /coordinator crashed/);
  await assert.rejects(
    driveMigration(w.ports, { ...RUN, migrationId: "m-other" }),
    (e: unknown) => e instanceof FailClosed && e.code === "migration_in_progress",
  );
});
