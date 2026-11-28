// #3401 acceptance: "Directory schema allowlist excludes graph/private
// content." The shared D1 directory is the single shared durable-directory
// residency exception: its records hold pseudonymous subject keys and
// minimal binding, jurisdiction, and migration metadata only — no graph
// content, journals, blob bytes, or private profile data (see the hosted
// README's residency-exception disclosure).
//
// Oracle: migrations/0001_hosted_accounts.sql applied to an in-memory
// SQLite database must create exactly one table with exactly the
// allowlisted columns, and the record constructors must expose exactly
// the allowlisted fields. The allowlist itself lives in
// ../src/account-directory.ts (HOSTED_DIRECTORY_ALLOWLISTED_COLUMNS)
// so the migration, the query code, and this suite pin one list, and
// schema drift fails loudly instead of silently widening the shared
// exception.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
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

function freshDirectoryDb(): DatabaseSync {
  const db = new DatabaseSync(":memory:");
  db.exec(readFileSync(join(here, "..", "migrations", "0001_hosted_accounts.sql"), "utf8"));
  return db;
}

function tableNames(db: DatabaseSync): string[] {
  const rows = db
    .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT GLOB 'sqlite_*' ORDER BY name")
    .all() as { name: string }[];
  return rows.map((r) => r.name);
}

function columnNames(db: DatabaseSync, table: string): string[] {
  const rows = db.prepare(`PRAGMA table_xinfo(${table})`).all() as { name: string }[];
  return rows.map((r) => r.name);
}

function assertDirectorySchema(db: DatabaseSync): void {
  assert.deepEqual(tableNames(db), ["hosted_accounts"]);
  assert.deepEqual(columnNames(db, "hosted_accounts"), [...HOSTED_DIRECTORY_ALLOWLISTED_COLUMNS]);
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
      if (pattern.test(column)) {
        violations.push(`${table}.${column} matches ${pattern} (#3401: never in the shared directory)`);
        break;
      }
    }
  }
  return violations;
}

test("shared directory migration creates exactly the allowlisted table and columns", () => {
  const db = freshDirectoryDb();
  try {
    assertDirectorySchema(db);
  } finally {
    db.close();
  }
});

test("schema allowlist rejects a non-system table with a sqlite-like name", () => {
  const db = freshDirectoryDb();
  try {
    db.exec("CREATE TABLE sqliteXprivate (private_profile TEXT)");
    assert.throws(() => assertDirectorySchema(db));
  } finally {
    db.close();
  }
});

test("schema allowlist rejects generated columns", () => {
  const db = freshDirectoryDb();
  try {
    db.exec("ALTER TABLE hosted_accounts ADD COLUMN private_profile TEXT GENERATED ALWAYS AS (subject_key) VIRTUAL");
    assert.throws(() => assertDirectorySchema(db));
  } finally {
    db.close();
  }
});

test("no graph, journal, blob, or private-profile column in the shared schema", () => {
  const db = freshDirectoryDb();
  try {
    const columns = tableNames(db).flatMap((t) => columnNames(db, t).map((c) => [t, c] as [string, string]));
    assert.deepEqual(neverSharedViolations(columns), []);
  } finally {
    db.close();
  }
});

test("the never-shared guard actually bites", () => {
  // If the guard could not see a planted violation, the pin above
  // would be vacuous. Each of these is a #3401 breach.
  const planted: [string, string][] = [
    ["hosted_accounts", "graph_edges"],
    ["hosted_accounts", "journal_entry"],
    ["hosted_accounts", "blob_bytes"],
    ["hosted_accounts", "private_profile"],
    ["hosted_accounts", "email_address"],
  ];
  const violations = neverSharedViolations(planted);
  assert.equal(violations.length, planted.length, violations.join("; "));
  // ...and the sanctioned pseudonymous identifiers stay legal.
  assert.deepEqual(
    neverSharedViolations([
      ["hosted_accounts", "subject_key"],
      ["hosted_accounts", "binding_id"],
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
  assert.deepEqual(Object.keys(legacy).sort(), ["bindingId", "classification", "jurisdiction", "subjectKey"]);
});
