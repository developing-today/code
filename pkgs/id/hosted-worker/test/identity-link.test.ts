// H2 of #9086 (issue #14260): the SSO relying-party identity_link
// table + link/unlink API.
//
// Oracle: doc/2026-11-24T04-50-00Z_design_sso-relying-party-9086
// section 4.3 (link table home + semantics: link ceremony, last-door
// guard, history retention, DB continuity) and section 5 slice H2
// acceptance ("link/unlink round-trip tests incl. guard refusal,
// history retention, and DB-continuity proof — tenant rows untouched
// across relink").
//
// Harness style follows test/tenant-ops.test.ts: the real tenancy
// migrations applied to an in-memory node:sqlite database, the
// store exercised through the same fakeSql seam the TenantOpsDO
// tests use — never a copy of the schema.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  linkDoor,
  listIdentityLinks,
  resolveAccountIdByDoor,
  unlinkDoor,
  type IdentityLinkSql,
  type LinkSession,
} from "../src/identity-link.ts";

const here = dirname(fileURLToPath(import.meta.url));
const migrationsDir = join(here, "..", "tenancy", "migrations");

function freshTenancyDb(): DatabaseSync {
  const db = new DatabaseSync(":memory:");
  db.exec("PRAGMA foreign_keys = ON");
  const files = readdirSync(migrationsDir)
    .filter((f) => f.endsWith(".sql"))
    .sort();
  for (const f of files) db.exec(readFileSync(join(migrationsDir, f), "utf8"));
  return db;
}

function fakeSql(db: DatabaseSync): IdentityLinkSql {
  return {
    exec(query: string, ...params: (string | null)[]) {
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

const ACCOUNT = "acct-1";
const ACCOUNT2 = "acct-2";
const BINDING = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BINDING2 = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const T0 = "2026-11-24T00:00:00.000Z";
const T1 = "2026-11-24T01:00:00.000Z";
const T2 = "2026-11-24T02:00:00.000Z";

function seedAccount(db: DatabaseSync, accountId: string, bindingId: string) {
  db.prepare(
    "INSERT INTO account (account_id, sub, email_contact, created_at, state, tier) VALUES (?, ?, NULL, ?, 'active', 'standard')",
  ).run(accountId, `provider-subject-for-${accountId}`, T0);
  db.prepare(
    "INSERT INTO binding (binding_id, account_id, iroh_id, status, bound_at, superseded_by) VALUES (?, ?, 'iroh-public', 'active', ?, NULL)",
  ).run(bindingId, accountId, T0);
}

function setup(): { db: DatabaseSync; sql: IdentityLinkSql; seq: { n: number } } {
  const db = freshTenancyDb();
  seedAccount(db, ACCOUNT, BINDING);
  seedAccount(db, ACCOUNT2, BINDING2);
  return { db, sql: fakeSql(db), seq: { n: 0 } };
}

function deps(seq: { n: number }, at: string) {
  return { now: () => at, newId: () => `id-${++seq.n}` };
}

const SESSION: LinkSession = { accountId: ACCOUNT, bindingId: BINDING };
const SESSION2: LinkSession = { accountId: ACCOUNT2, bindingId: BINDING2 };

interface EventRow {
  link_id: string;
  account_id: string;
  door: string;
  action: string;
  actor_binding_id: string;
  at: string;
}

function events(db: DatabaseSync): EventRow[] {
  return db
    .prepare(
      "SELECT link_id, account_id, door, action, actor_binding_id, at FROM identity_link_event ORDER BY at, event_id",
    )
    .all() as unknown as EventRow[];
}

test("migration 0002 creates identity_link + identity_link_event with the section 4.3 shape", () => {
  const db = freshTenancyDb();
  const version = db.prepare("SELECT value FROM meta WHERE name = 'schema_version'").get() as {
    value: string;
  };
  assert.equal(version.value, "2");
  const cols = (t: string) =>
    (db.prepare(`PRAGMA table_info(${t})`).all() as { name: string }[]).map((r) => r.name);
  assert.deepEqual(cols("identity_link"), [
    "link_id",
    "account_id",
    "door",
    "provider_sub",
    "linked_at",
    "linked_by",
    "unlinked_at",
  ]);
  assert.deepEqual(cols("identity_link_event"), [
    "event_id",
    "link_id",
    "account_id",
    "door",
    "action",
    "actor_binding_id",
    "at",
  ]);
  // The door vocabulary is closed, and (door, provider_sub) is
  // unique across ALL rows, history included.
  seedAccount(db, ACCOUNT, BINDING);
  const insert = db.prepare(
    "INSERT INTO identity_link (link_id, account_id, door, provider_sub, linked_at, linked_by) VALUES (?, ?, ?, ?, ?, ?)",
  );
  assert.throws(() => insert.run("l0", ACCOUNT, "carrier-pigeon", "s", T0, BINDING), /CHECK/);
  insert.run("l1", ACCOUNT, "github", "sub-1", T0, BINDING);
  assert.throws(() => insert.run("l2", ACCOUNT, "github", "sub-1", T0, BINDING), /UNIQUE/);
});

test("link ceremony: session + new-door proof links a door; resolve finds the account", () => {
  const { db, sql, seq } = setup();
  const result = linkDoor(sql, SESSION, { door: "github", providerSub: "gh-42" }, deps(seq, T0));
  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.equal(result.link.accountId, ACCOUNT);
  assert.equal(result.link.linkedBy, BINDING);
  assert.equal(result.link.unlinkedAt, null);
  assert.equal(resolveAccountIdByDoor(sql, "github", "gh-42"), ACCOUNT);
  assert.equal(resolveAccountIdByDoor(sql, "google", "gh-42"), null);
  assert.equal(resolveAccountIdByDoor(sql, "github", "never-linked"), null);
  const evts = events(db);
  assert.equal(evts.length, 1);
  assert.equal(evts[0]!.action, "linked");
  assert.equal(evts[0]!.door, "github");
  assert.equal(evts[0]!.actor_binding_id, BINDING);
  assert.equal(evts[0]!.at, T0);
});

test("link refusals: bad door, unknown account, foreign or superseded actor binding, duplicate", () => {
  const { db, sql, seq } = setup();
  assert.deepEqual(
    linkDoor(sql, SESSION, { door: "carrier-pigeon", providerSub: "x" }, deps(seq, T0)),
    { ok: false, reason: "bad_door" },
  );
  assert.deepEqual(
    linkDoor(
      sql,
      { accountId: "acct-missing", bindingId: BINDING },
      { door: "github", providerSub: "x" },
      deps(seq, T0),
    ),
    { ok: false, reason: "account_not_found" },
  );
  // A binding of ANOTHER account performs no ceremony here.
  assert.deepEqual(
    linkDoor(
      sql,
      { accountId: ACCOUNT, bindingId: BINDING2 },
      { door: "github", providerSub: "x" },
      deps(seq, T0),
    ),
    { ok: false, reason: "actor_not_account_binding" },
  );
  // Nor does the account's own binding once superseded.
  db.prepare("UPDATE binding SET status = 'superseded' WHERE binding_id = ?").run(BINDING2);
  assert.deepEqual(
    linkDoor(sql, SESSION2, { door: "github", providerSub: "x" }, deps(seq, T0)),
    { ok: false, reason: "actor_not_account_binding" },
  );
  const first = linkDoor(sql, SESSION, { door: "github", providerSub: "gh-42" }, deps(seq, T0));
  assert.equal(first.ok, true);
  assert.deepEqual(
    linkDoor(sql, SESSION, { door: "github", providerSub: "gh-42" }, deps(seq, T1)),
    { ok: false, reason: "already_linked" },
  );
  // Refusals wrote nothing: exactly one link row, one event.
  assert.equal(listIdentityLinks(sql, ACCOUNT).length, 1);
  assert.equal(events(db).length, 1);
});

test("unlink round-trip: row retained as history, resolve stops, event recorded", () => {
  const { db, sql, seq } = setup();
  linkDoor(sql, SESSION, { door: "github", providerSub: "gh-42" }, deps(seq, T0));
  linkDoor(sql, SESSION, { door: "google", providerSub: "go-7" }, deps(seq, T0));
  const result = unlinkDoor(sql, SESSION, "github", deps(seq, T1));
  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.equal(result.unlinked.unlinkedAt, T1);
  assert.equal(result.replacement, null);
  assert.equal(resolveAccountIdByDoor(sql, "github", "gh-42"), null);
  assert.equal(resolveAccountIdByDoor(sql, "google", "go-7"), ACCOUNT);
  const links = listIdentityLinks(sql, ACCOUNT);
  assert.equal(links.length, 2, "history rows are retained, never hard-deleted");
  assert.equal(links.find((l) => l.door === "github")!.unlinkedAt, T1);
  const evts = events(db);
  assert.deepEqual(
    evts.map((e) => `${e.action}:${e.door}`),
    ["linked:github", "linked:google", "unlinked:github"],
  );
  // Unlinking an already-unlinked (or never-linked) door refuses.
  assert.deepEqual(unlinkDoor(sql, SESSION, "github", deps(seq, T2)), {
    ok: false,
    reason: "link_not_active",
  });
  assert.deepEqual(unlinkDoor(sql, SESSION, "microsoft", deps(seq, T2)), {
    ok: false,
    reason: "link_not_active",
  });
});

test("last-door guard: the single active link cannot be unlinked without a replacement", () => {
  const { db, sql, seq } = setup();
  linkDoor(sql, SESSION, { door: "github", providerSub: "gh-42" }, deps(seq, T0));
  assert.deepEqual(unlinkDoor(sql, SESSION, "github", deps(seq, T1)), {
    ok: false,
    reason: "last_door_guard",
  });
  // The refusal changed nothing: the door still opens the account,
  // and no audit event was written for a change that did not happen.
  assert.equal(resolveAccountIdByDoor(sql, "github", "gh-42"), ACCOUNT);
  assert.equal(events(db).length, 1);
});

test("last-door guard: unlink with a replacement in the same operation succeeds", () => {
  const { db, sql, seq } = setup();
  linkDoor(sql, SESSION, { door: "github", providerSub: "gh-42" }, deps(seq, T0));
  const result = unlinkDoor(sql, SESSION, "github", deps(seq, T1), {
    door: "google",
    providerSub: "go-7",
  });
  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.equal(result.unlinked.door, "github");
  assert.equal(result.replacement?.door, "google");
  assert.equal(resolveAccountIdByDoor(sql, "github", "gh-42"), null);
  assert.equal(resolveAccountIdByDoor(sql, "google", "go-7"), ACCOUNT);
  assert.deepEqual(
    events(db).map((e) => `${e.action}:${e.door}`),
    ["linked:github", "linked:google", "unlinked:github"],
  );
  // A refused replacement aborts the swap untouched: claiming a
  // door-identity another account holds leaves the last door active.
  const claimed = linkDoor(sql, SESSION2, { door: "microsoft", providerSub: "ms-1" }, deps(seq, T1));
  assert.equal(claimed.ok, true);
  const swap = unlinkDoor(sql, SESSION, "google", deps(seq, T2), {
    door: "microsoft",
    providerSub: "ms-1",
  });
  assert.deepEqual(swap, { ok: false, reason: "door_identity_claimed" });
  assert.equal(resolveAccountIdByDoor(sql, "google", "go-7"), ACCOUNT);
});

test("a door-identity belongs to one account ever, even after unlink", () => {
  const { sql, seq } = setup();
  linkDoor(sql, SESSION, { door: "google", providerSub: "go-7" }, deps(seq, T0));
  linkDoor(sql, SESSION, { door: "github", providerSub: "gh-42" }, deps(seq, T0));
  unlinkDoor(sql, SESSION, "google", deps(seq, T1));
  assert.deepEqual(
    linkDoor(sql, SESSION2, { door: "google", providerSub: "go-7" }, deps(seq, T2)),
    { ok: false, reason: "door_identity_claimed" },
  );
});

test("relink after unlink reactivates the retained history row", () => {
  const { db, sql, seq } = setup();
  linkDoor(sql, SESSION, { door: "github", providerSub: "gh-42" }, deps(seq, T0));
  linkDoor(sql, SESSION, { door: "google", providerSub: "go-7" }, deps(seq, T0));
  unlinkDoor(sql, SESSION, "github", deps(seq, T1));
  const relinked = linkDoor(sql, SESSION, { door: "github", providerSub: "gh-42" }, deps(seq, T2));
  assert.equal(relinked.ok, true);
  const rows = listIdentityLinks(sql, ACCOUNT).filter((l) => l.door === "github");
  assert.equal(rows.length, 1, "still exactly one row for the door-identity");
  assert.equal(rows[0]!.unlinkedAt, null);
  assert.equal(rows[0]!.linkedAt, T2);
  assert.equal(resolveAccountIdByDoor(sql, "github", "gh-42"), ACCOUNT);
  assert.deepEqual(
    events(db)
      .filter((e) => e.door === "github")
      .map((e) => e.action),
    ["linked", "unlinked", "linked"],
  );
});

test("DB continuity: tenant rows are untouched across a relink", () => {
  const { db, sql, seq } = setup();
  // Tenant-side state keyed off account_id/binding_id (design
  // section 4.3): a tenant_db row, role grants, a credential row.
  db.prepare(
    "INSERT INTO tenant_db (tenant_id, participant_kind, jurisdiction, backend, state, schema_version, created_at) VALUES (?, 'user_hosted', 'us', 'turso', 'active', 1, ?)",
  ).run(BINDING, T0);
  db.prepare(
    "INSERT INTO role_grant (binding_id, role, scope, granted_at, granted_by) VALUES (?, 'owner', '*', ?, 'system')",
  ).run(BINDING, T0);
  db.prepare(
    "INSERT INTO credential (credential_id, tenant_id, audience, scope, issued_at, expires_at, issuer, jti) VALUES ('cred-1', ?, 'db-1', 'read', ?, ?, 'worker', 'jti-1')",
  ).run(BINDING, T0, T2);
  const snapshot = () => ({
    account: db.prepare("SELECT * FROM account ORDER BY account_id").all(),
    binding: db.prepare("SELECT * FROM binding ORDER BY binding_id").all(),
    tenantDb: db.prepare("SELECT * FROM tenant_db ORDER BY tenant_id").all(),
    roleGrant: db.prepare("SELECT * FROM role_grant ORDER BY binding_id, role, scope").all(),
    credential: db.prepare("SELECT * FROM credential ORDER BY credential_id").all(),
  });
  linkDoor(sql, SESSION, { door: "github", providerSub: "gh-42" }, deps(seq, T0));
  const before = snapshot();
  // The relink: swap the account's only door github -> google.
  const swap = unlinkDoor(sql, SESSION, "github", deps(seq, T1), {
    door: "google",
    providerSub: "go-7",
  });
  assert.equal(swap.ok, true);
  assert.deepEqual(snapshot(), before, "no tenant row changed across the relink");
});
