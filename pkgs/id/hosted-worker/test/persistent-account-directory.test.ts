import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { DatabaseSync } from "node:sqlite";
import { test } from "node:test";
import {
  PersistentHostedAccountDirectory,
  type HostedDirectoryDatabase,
  type HostedDirectoryStatement,
} from "../src/persistent-account-directory.ts";
import { FailClosed } from "../src/routing.ts";

const BINDING = "0123456789abcdef0123456789abcdef";
const OTHER_BINDING = "fedcba9876543210fedcba9876543210";

function sqliteD1(db: DatabaseSync): HostedDirectoryDatabase {
  return {
    prepare(query: string): HostedDirectoryStatement {
      let values: Array<string | number | null> = [];
      return {
        bind(...bound) {
          values = bound;
          return this;
        },
        async first<T extends Record<string, unknown>>() {
          // Yield before each statement so concurrent signup calls can both
          // observe a missing subject before SQLite serializes their inserts.
          await Promise.resolve();
          return (db.prepare(query).get(...values) as T | undefined) ?? null;
        },
      };
    },
  };
}

async function createTestDirectory() {
  const db = new DatabaseSync(":memory:");
  const schema = await readFile(
    new URL("../migrations/0001_hosted_accounts.sql", import.meta.url),
    "utf8",
  );
  db.exec(schema);
  return { db, directory: new PersistentHostedAccountDirectory(sqliteD1(db)) };
}

test("persistent duplicate signup returns the stored row without changing its policy", async () => {
  const { db, directory: accountDirectory } = await createTestDirectory();
  try {
    const first = await accountDirectory.signup({
      subjectKey: "subject-1",
      bindingId: BINDING,
      jurisdiction: "eu",
    });
    const duplicate = await accountDirectory.signup({
      subjectKey: "subject-1",
      bindingId: BINDING,
      jurisdiction: "us",
    });
    assert.deepEqual(duplicate, first);
    assert.equal(
      db.prepare("SELECT COUNT(*) AS count FROM hosted_accounts").get()?.count,
      1,
    );
  } finally {
    db.close();
  }
});

test("concurrent persistent signups for one subject preserve exactly one row", async () => {
  const { db, directory: accountDirectory } = await createTestDirectory();
  try {
    const [left, right] = await Promise.all([
      accountDirectory.signup({
        subjectKey: "subject-race",
        bindingId: BINDING,
        jurisdiction: "eu",
      }),
      accountDirectory.signup({
        subjectKey: "subject-race",
        bindingId: BINDING,
        jurisdiction: "us",
      }),
    ]);
    assert.deepEqual(left, right);
    assert.equal(
      db.prepare("SELECT COUNT(*) AS count FROM hosted_accounts").get()?.count,
      1,
    );
    assert.ok(["eu", "us"].includes(left.jurisdiction));
  } finally {
    db.close();
  }
});

test("persistent signup rejects conflicting subject and binding mappings", async () => {
  const { db, directory: accountDirectory } = await createTestDirectory();
  try {
    await accountDirectory.signup({
      subjectKey: "subject-1",
      bindingId: BINDING,
      jurisdiction: "eu",
    });
    await assert.rejects(
      accountDirectory.signup({
        subjectKey: "subject-1",
        bindingId: OTHER_BINDING,
        jurisdiction: "eu",
      }),
      (error: unknown) =>
        error instanceof FailClosed &&
        error.code === "duplicate_subject_conflict",
    );
    await assert.rejects(
      accountDirectory.signup({
        subjectKey: "subject-2",
        bindingId: BINDING,
        jurisdiction: "eu",
      }),
      (error: unknown) =>
        error instanceof FailClosed && error.code === "binding_conflict",
    );
    assert.equal(
      db.prepare("SELECT COUNT(*) AS count FROM hosted_accounts").get()?.count,
      1,
    );
  } finally {
    db.close();
  }
});

test("persistent signup fails closed before creating a row for invalid jurisdiction", async () => {
  const { db, directory: accountDirectory } = await createTestDirectory();
  try {
    await assert.rejects(
      accountDirectory.signup({
        subjectKey: "subject-1",
        bindingId: BINDING,
        jurisdiction: "unknown",
      }),
      (error: unknown) =>
        error instanceof FailClosed &&
        error.code === "unsupported_jurisdiction",
    );
    assert.equal(
      db.prepare("SELECT COUNT(*) AS count FROM hosted_accounts").get()?.count,
      0,
    );
  } finally {
    db.close();
  }
});

test("directory migration CHECK constraints reject malformed identity and routing metadata", async () => {
  const { db } = await createTestDirectory();
  const insert = db.prepare(
    "INSERT INTO hosted_accounts (subject_key, binding_id, jurisdiction, jurisdiction_pending, migration_id, phase, routing_epoch) VALUES (?, ?, ?, ?, ?, ?, ?)",
  );
  const valid = ["subject", BINDING, "us", null, null, "stable", 0] as const;
  const cases = [
    ["blank subject", ["   ", ...valid.slice(1)]],
    ["short binding id", ["subject", "0123456789abcdef0123456789abcde", ...valid.slice(2)]],
    ["non-hex binding id", ["subject", "g" + BINDING.slice(1), ...valid.slice(2)]],
    ["unsupported jurisdiction", ["subject", BINDING, "unknown", ...valid.slice(3)]],
    ["negative routing epoch", ["subject", BINDING, "us", null, null, "stable", -1]],
  ] as const;

  try {
    for (const [name, values] of cases) {
      assert.throws(() => insert.run(...values), undefined, name);
      db.exec("DELETE FROM hosted_accounts");
    }
  } finally {
    db.close();
  }
});

test("persistent reads reject records outside the current stable account contract", async () => {
  const { db, directory: accountDirectory } = await createTestDirectory();
  const invalidRows = [
    ["pending jurisdiction", "us", null, "stable", 0],
    ["migration id", null, "migration-1", "stable", 0],
    ["non-zero routing epoch", null, null, "stable", 1],
    ["unknown phase", null, null, "moving", 0],
  ] as const;

  try {
    for (const [index, [name, pending, migrationId, phase, epoch]] of invalidRows.entries()) {
      db.prepare(
        "INSERT INTO hosted_accounts (subject_key, binding_id, jurisdiction, jurisdiction_pending, migration_id, phase, routing_epoch) VALUES (?, ?, ?, ?, ?, ?, ?)",
      ).run(`subject-${index}`, BINDING, "us", pending, migrationId, phase, epoch);

      await assert.rejects(
        accountDirectory.get(`subject-${index}`),
        (error: unknown) =>
          error instanceof FailClosed && error.code === "invalid_directory_record",
        name,
      );
      db.exec("DELETE FROM hosted_accounts");
    }
  } finally {
    db.close();
  }
});

test("hosted README discloses the shared directory residency exception and its limits", async () => {
  const readme = await readFile(new URL("../README.md", import.meta.url), "utf8");

  // Oracle: the disclosure checklist in the jurisdiction-aware store design
  // requires the shared D1 exception, the pseudonymous-data caveat, and the
  // separate KV, logging, request-processing, and backup residency gaps.
  assert.match(readme, /single shared durable-directory residency exception/i);
  assert.match(readme, /hashed subject.{0,40}potentially personal data/i);
  assert.match(readme, /KV caches/i);
  assert.match(readme, /logs/i);
  assert.match(readme, /request processing/i);
  assert.match(readme, /backup placement/i);
});
