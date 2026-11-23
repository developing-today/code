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
