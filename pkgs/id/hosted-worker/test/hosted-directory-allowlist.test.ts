// #3401 acceptance: "Directory schema allowlist excludes graph/private
// content." The shared D1 directory is the single shared durable-directory
// residency exception: its records hold pseudonymous subject keys and
// minimal binding, jurisdiction, and migration metadata only — no graph
// content, journals, blob bytes, or private profile data (see the hosted
// README's residency-exception disclosure).
//
// Oracle: every hosted-worker migration applied to an in-memory SQLite
// database must create exactly the expected tables and columns. The
// hosted_accounts allowlist itself lives in ../src/account-directory.ts
// (HOSTED_DIRECTORY_ALLOWLISTED_COLUMNS) and is shared with the query
// layer; the legacy-inventory and migration-lease tables have separate
// minimal schemas pinned here. Record constructors expose only their
// declared fields, so schema drift cannot silently widen shared storage.
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import {
  createHostedAccountRecord,
  HOSTED_DIRECTORY_ALLOWLISTED_COLUMNS,
  inventoryLegacyHostedAccount,
} from "../src/account-directory.ts";

const here = dirname(fileURLToPath(import.meta.url));

const EXPECTED_TABLE_COLUMNS: Record<string, readonly string[]> = {
  hosted_accounts: HOSTED_DIRECTORY_ALLOWLISTED_COLUMNS,
  legacy_hosted_accounts: ["subject_key", "binding_id", "classification"],
  migration_leases: ["subject_key", "owner", "token", "expires_ms"],
};

function freshDirectoryDb(): DatabaseSync {
  const db = new DatabaseSync(":memory:");
  const migrations = join(here, "..", "migrations");
  for (const file of readdirSync(migrations).filter((name) => name.endsWith(".sql")).sort()) {
    db.exec(readFileSync(join(migrations, file), "utf8"));
  }
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

// Graph content, journals, blob bytes, and private profile data must
// never enter the shared schema. Pseudonymous subject/binding keys are
// the sanctioned identifiers (like content_hash in the tenancy never-
// central list), so /key/ and /binding/ are deliberately absent here.
const NEVER_SHARED_PATTERNS: RegExp[] = [
  /graph/,
  /journal/,
  /blob/,
  /content/,
  /profile/,
  /email/,
  /token/,
  /secret/,
  /passw/,
  /payload/,
  /bytes/,
  /body/,
  /event/,
  /plaintext/,
  /ciphertext/,
];

function neverSharedViolations(columns: [string, string][]): string[] {
  const violations: string[] = [];
  for (const [table, column] of columns) {
    for (const pattern of NEVER_SHARED_PATTERNS) {
      if (table === "migration_leases" && column === "token") continue;
      if (pattern.test(column)) {
        violations.push(`${table}.${column} matches ${pattern} (#3401: never in the shared directory)`);
        break;
      }
    }
  }
  return violations;
}

test("hosted-worker migrations create only the explicitly allowlisted tables and columns", () => {
  const db = freshDirectoryDb();
  try {
    assert.deepEqual(tableNames(db), Object.keys(EXPECTED_TABLE_COLUMNS).sort());
    for (const [table, expectedColumns] of Object.entries(EXPECTED_TABLE_COLUMNS)) {
      assert.deepEqual(columnNames(db, table), [...expectedColumns], `${table} schema drifted`);
    }
  } finally {
    db.close();
  }
});

test("no graph, journal, blob, or private-profile column in hosted-worker schemas", () => {
  const db = freshDirectoryDb();
  try {
    const columns = tableNames(db).flatMap((t) =>
      columnNames(db, t).map((c) => [t, c] as [string, string]),
    );
    assert.deepEqual(neverSharedViolations(columns), []);
  } finally {
    db.close();
  }
});

test("the never-shared guard actually bites and permits only the lease fencing token", () => {
  // If the guard could not see a planted violation, the pin above
  // would be vacuous. Each of these is a #3401 breach.
  const planted: [string, string][] = [
    ["hosted_accounts", "graph_edges"],
    ["hosted_accounts", "journal_entry"],
    ["hosted_accounts", "blob_bytes"],
    ["hosted_accounts", "private_profile"],
    ["hosted_accounts", "email_address"],
    ["hosted_accounts", "api_token"],
  ];
  const violations = neverSharedViolations(planted);
  assert.equal(violations.length, planted.length, violations.join("; "));
  // ...and the sanctioned pseudonymous identifiers stay legal.
  assert.deepEqual(
    neverSharedViolations([
      ["hosted_accounts", "subject_key"],
      ["hosted_accounts", "binding_id"],
      // This is an integer fencing counter, not an authentication token.
      ["migration_leases", "token"],
    ]),
    [],
  );
});

test("record constructors expose exactly the allowlisted fields", () => {
  const record = createHostedAccountRecord({
    subjectKey: "subject-1",
    bindingId: "0123456789abcdef0123456789abcdef",
    jurisdiction: "eu",
  });
  assert.deepEqual(Object.keys(record).sort(), [
    "bindingId",
    "jurisdiction",
    "jurisdictionPending",
    "migrationId",
    "phase",
    "routingEpoch",
    "subjectKey",
  ]);
  const legacy = inventoryLegacyHostedAccount({
    subjectKey: "subject-1",
    bindingId: "0123456789abcdef0123456789abcdef",
  });
  assert.deepEqual(Object.keys(legacy).sort(), [
    "bindingId",
    "classification",
    "jurisdiction",
    "subjectKey",
  ]);
});
