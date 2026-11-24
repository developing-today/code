// E1 of #9083 (issue #12796): the central tenancy DB schema baseline,
// as amended by H2 of #9086 (issue #14260).
//
// Oracle: the hosted-tenancy design record
// (doc/2026-11-17T16-55-00Z_design_hosted-tenancy-architecture)
// section 2.1 (the schema this migration must implement) and
// section 2.3 (the "never central" list this suite guards); for the
// identity_link tables, the SSO relying-party design record
// (doc/2026-11-24T04-50-00Z_design_sso-relying-party-9086) section
// 4.3 — migration 0002 adds them and bumps schema_version to 2.
//
// The suite applies every migration under ../tenancy/migrations to
// an in-memory SQLite database (the tenancy DB is a Turso/SQLite
// database) and pins three things:
//
//   1. the schema of record's shape, table for table, column for column;
//   2. the never-central invariant: no column may hold content, a
//      key, or a token value -- tokens are metadata only (a `jti`
//      revocation handle), keys appear only as the public `iroh_id`;
//   3. the peer principle: hosted, native (Rust-binary), self-hosted
//      and public tenants are one `participant_kind` vocabulary in
//      one table -- no hosted-only schema objects, no special-casing.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const migrationsDir = join(here, "..", "tenancy", "migrations");

function migrationSql(): string {
  const files = readdirSync(migrationsDir)
    .filter((f) => f.endsWith(".sql"))
    .sort();
  assert.ok(files.length > 0, "tenancy/migrations must contain the baseline migration");
  return files.map((f) => readFileSync(join(migrationsDir, f), "utf8")).join("\n");
}

function freshTenancyDb(): DatabaseSync {
  const db = new DatabaseSync(":memory:");
  db.exec("PRAGMA foreign_keys = ON");
  db.exec(migrationSql());
  return db;
}

function tableNames(db: DatabaseSync): string[] {
  const rows = db
    .prepare(
      "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .all() as { name: string }[];
  return rows.map((r) => r.name);
}

function columnNames(db: DatabaseSync, table: string): string[] {
  const rows = db.prepare(`PRAGMA table_info(${table})`).all() as { name: string }[];
  return rows.map((r) => r.name);
}

function allColumns(db: DatabaseSync): [string, string][] {
  return tableNames(db).flatMap((t) => columnNames(db, t).map((c) => [t, c] as [string, string]));
}

// The section 2.1 baseline, column for column, in DDL order.
const EXPECTED_SCHEMA: Record<string, string[]> = {
  account: ["account_id", "sub", "email_contact", "created_at", "state", "tier", "meter_rollup"],
  binding: ["binding_id", "account_id", "iroh_id", "status", "bound_at", "superseded_by"],
  credential: [
    "credential_id",
    "tenant_id",
    "audience",
    "scope",
    "issued_at",
    "expires_at",
    "revoked_at",
    "issuer",
    "jti",
  ],
  identity_link: ["link_id", "account_id", "door", "provider_sub", "linked_at", "linked_by", "unlinked_at"],
  identity_link_event: ["event_id", "link_id", "account_id", "door", "action", "actor_binding_id", "at"],
  meta: ["name", "value"],
  peer_policy: [
    "peer_id",
    "tenant_id",
    "class",
    "operations",
    "relay_granted",
    "scope",
    "policy_version",
    "valid_from",
    "expires_at",
  ],
  publish_record: [
    "publish_id",
    "tenant_id",
    "content_hash",
    "kind",
    "signed_artifact",
    "published_at",
    "retracted_at",
    "policy_version",
  ],
  role_grant: ["binding_id", "role", "scope", "granted_at", "granted_by", "revoked_at"],
  tenant_db: [
    "tenant_id",
    "participant_kind",
    "turso_db_id",
    "turso_db_url",
    "jurisdiction",
    "backend",
    "state",
    "schema_version",
    "created_at",
    "deleted_at",
  ],
};

// Section 2.3 as column-name rules. A column whose name says it holds
// a token value, a key, a secret, or content bytes must never appear
// in the central schema. `content_hash` is the one sanctioned
// exception: it is a content ADDRESS (the publish grant), not content.
const NEVER_CENTRAL_PATTERNS: RegExp[] = [
  /token/,
  /key/,
  /secret/,
  /passw/,
  /body/,
  /bytes/,
  /payload/,
  /plaintext/,
  /ciphertext/,
  /content/,
];
const NEVER_CENTRAL_ALLOWLIST = new Set(["content_hash"]);

function neverCentralViolations(columns: [string, string][]): string[] {
  const violations: string[] = [];
  for (const [table, column] of columns) {
    if (NEVER_CENTRAL_ALLOWLIST.has(column)) continue;
    for (const pattern of NEVER_CENTRAL_PATTERNS) {
      if (pattern.test(column)) {
        violations.push(`${table}.${column} matches ${pattern} (design section 2.3: never central)`);
        break;
      }
    }
  }
  return violations;
}

test("migrations apply cleanly and create exactly the schema-of-record tables", () => {
  const db = freshTenancyDb();
  assert.deepEqual(tableNames(db), Object.keys(EXPECTED_SCHEMA).sort());
  const row = db.prepare("SELECT value FROM meta WHERE name = 'schema_version'").get() as {
    value: string;
  };
  assert.equal(row.value, "2");
});

test("schema matches the section 2.1 baseline (plus the H2 link tables) column for column", () => {
  const db = freshTenancyDb();
  for (const [table, expected] of Object.entries(EXPECTED_SCHEMA)) {
    assert.deepEqual(columnNames(db, table), expected, `table ${table}`);
  }
});

test("never central: no content, key, or token column in the schema", () => {
  const db = freshTenancyDb();
  assert.deepEqual(neverCentralViolations(allColumns(db)), []);
});

test("the never-central guard actually bites", () => {
  // If the guard could not see a planted violation, the test above
  // would be vacuous. Each of these is a section 2.3 breach.
  const planted: [string, string][] = [
    ["credential", "token_value"],
    ["account", "private_key"],
    ["tenant_db", "note_body"],
    ["publish_record", "blob_bytes"],
    ["account", "api_secret"],
  ];
  const violations = neverCentralViolations(planted);
  assert.equal(violations.length, planted.length, violations.join("; "));
  // ...and the sanctioned reference columns stay legal.
  assert.deepEqual(
    neverCentralViolations([
      ["publish_record", "content_hash"],
      ["binding", "iroh_id"],
      ["credential", "jti"],
    ]),
    [],
  );
});

test("peer principle: one participant_kind vocabulary, no hosted-only objects", () => {
  const db = freshTenancyDb();
  const row = db
    .prepare("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'tenant_db'")
    .get() as { sql: string };
  for (const kind of ["user_hosted", "user_native", "self_hosted_node", "public"]) {
    assert.ok(row.sql.includes(`'${kind}'`), `participant_kind must include ${kind}`);
  }
  // participant_kind lives in exactly one table: kinds are
  // configuration on the shared registry row, not per-kind tables.
  const withKind = tableNames(db).filter((t) => columnNames(db, t).includes("participant_kind"));
  assert.deepEqual(withKind, ["tenant_db"]);
  // No schema object is named for one participant kind.
  const objects = db
    .prepare("SELECT name FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'")
    .all() as { name: string }[];
  for (const { name } of objects) {
    assert.ok(!/hosted/.test(name), `schema object ${name} special-cases hosted tenants`);
  }
});

const ACCOUNT = "acct-1";
const BINDING_A = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BINDING_B = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const NOW = "2026-11-23T00:00:00Z";

function insertAccount(db: DatabaseSync) {
  db.prepare(
    "INSERT INTO account (account_id, sub, email_contact, created_at, state, tier) VALUES (?, ?, ?, ?, ?, ?)",
  ).run(ACCOUNT, "provider-subject-1", null, NOW, "active", "standard");
}

function insertBinding(db: DatabaseSync, bindingId: string, status: string, supersededBy: string | null) {
  db.prepare(
    "INSERT INTO binding (binding_id, account_id, iroh_id, status, bound_at, superseded_by) VALUES (?, ?, ?, ?, ?, ?)",
  ).run(bindingId, ACCOUNT, "iroh-public-key", status, NOW, supersededBy);
}

test("binding rule: at most one active binding per account, rebind chain allowed", () => {
  const db = freshTenancyDb();
  insertAccount(db);
  insertBinding(db, BINDING_A, "active", null);
  assert.throws(() => insertBinding(db, BINDING_B, "active", null), /UNIQUE/);
  // The rebind flow the design's superseded_by chain exists for: the
  // new binding starts rebind_pending, the old one is superseded by
  // it, and only then may the new one go active.
  insertBinding(db, BINDING_B, "rebind_pending", null);
  db.prepare("UPDATE binding SET status = 'superseded', superseded_by = ? WHERE binding_id = ?").run(
    BINDING_B,
    BINDING_A,
  );
  db.prepare("UPDATE binding SET status = 'active' WHERE binding_id = ?").run(BINDING_B);
});

test("closed vocabularies and foreign keys are enforced", () => {
  const db = freshTenancyDb();
  assert.throws(
    () =>
      db
        .prepare(
          "INSERT INTO account (account_id, sub, created_at, state, tier) VALUES ('x', 's', ?, 'bogus', 't')",
        )
        .run(NOW),
    /CHECK/,
  );
  insertAccount(db);
  assert.throws(() => insertBinding(db, BINDING_A, "bogus", null), /CHECK/);
  assert.throws(
    () =>
      db
        .prepare("INSERT INTO binding (binding_id, account_id, iroh_id, status, bound_at) VALUES (?, 'nope', 'k', 'active', ?)")
        .run(BINDING_A, NOW),
    /FOREIGN KEY/,
  );
  assert.throws(
    () =>
      db
        .prepare(
          "INSERT INTO tenant_db (tenant_id, participant_kind, jurisdiction, backend, state, schema_version, created_at) VALUES (?, 'martian', 'us', 'turso', 'active', 1, ?)",
        )
        .run(BINDING_A, NOW),
    /CHECK/,
  );
  assert.throws(
    () =>
      db
        .prepare(
          "INSERT INTO tenant_db (tenant_id, participant_kind, jurisdiction, backend, state, schema_version, created_at) VALUES (?, 'user_hosted', 'apac', 'turso', 'active', 1, ?)",
        )
        .run(BINDING_A, NOW),
    /CHECK/,
  );
  assert.throws(
    () =>
      db
        .prepare(
          "INSERT INTO peer_policy (peer_id, tenant_id, class, operations, scope, policy_version) VALUES ('p', ?, 'buddy', '[]', 's', 1)",
        )
        .run(BINDING_A),
    /CHECK/,
  );
  assert.throws(
    () =>
      db
        .prepare(
          "INSERT INTO credential (credential_id, tenant_id, audience, scope, issued_at, expires_at, issuer, jti) VALUES ('c', ?, 'db', 'admin', ?, ?, 'w', 'j')",
        )
        .run(BINDING_A, NOW, NOW),
    /CHECK/,
  );
});

test("registry rows for every participant kind insert identically", () => {
  const db = freshTenancyDb();
  insertAccount(db);
  insertBinding(db, BINDING_A, "active", null);
  const insertTenant = db.prepare(
    "INSERT INTO tenant_db (tenant_id, participant_kind, jurisdiction, backend, state, schema_version, created_at) VALUES (?, ?, 'us', 'turso', 'active', 1, ?)",
  );
  for (const [i, kind] of ["user_hosted", "user_native", "self_hosted_node", "public"].entries()) {
    insertTenant.run(`${BINDING_A}-${i}`, kind, NOW);
  }
  const count = db.prepare("SELECT COUNT(*) AS n FROM tenant_db").get() as { n: number };
  assert.equal(count.n, 4);
});
