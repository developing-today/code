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
        async all<T extends Record<string, unknown>>() {
          await Promise.resolve();
          return db.prepare(query).all(...values) as T[];
        },
      };
    },
  };
}

async function createTestDirectory() {
  const db = new DatabaseSync(":memory:");
  for (const migration of ["0001_hosted_accounts.sql", "0002_legacy_hosted_accounts.sql"]) {
    const schema = await readFile(new URL(`../migrations/${migration}`, import.meta.url), "utf8");
    db.exec(schema);
  }
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

test("read-only binding lookup returns matching stable records and leaves storage unchanged", async () => {
  const { db, directory } = await createTestDirectory();
  try {
    const record = await directory.signup({
      subjectKey: "subject-lookup",
      bindingId: BINDING,
      jurisdiction: "eu",
    });
    const before = db.prepare("SELECT * FROM hosted_accounts").all();
    assert.deepEqual(await directory.findByBindingId(BINDING), record);
    assert.deepEqual(db.prepare("SELECT * FROM hosted_accounts").all(), before);
  } finally {
    db.close();
  }
});

test("read-only binding lookup returns missing for unknown rows", async () => {
  const { db, directory } = await createTestDirectory();
  try {
    assert.equal(await directory.findByBindingId(BINDING), undefined);
    assert.deepEqual(await directory.listLegacyInventory(), []);
  } finally {
    db.close();
  }
});

test("read-only binding lookup identifies a legacy-only row without mutation", async () => {
  const { db, directory } = await createTestDirectory();
  try {
    const legacy = await directory.inventoryLegacy({ subjectKey: "subject-legacy", bindingId: BINDING });
    const before = db.prepare("SELECT * FROM legacy_hosted_accounts").all();
    assert.equal(await directory.findByBindingId(BINDING), undefined);
    assert.deepEqual(await directory.findLegacyByBindingId(BINDING), legacy);
    await assert.rejects(
      directory.signup({ subjectKey: "subject-legacy", bindingId: BINDING, jurisdiction: "us" }),
      (error: unknown) => error instanceof FailClosed && error.code === "legacy_subject_requires_migration",
    );
    assert.deepEqual(db.prepare("SELECT * FROM legacy_hosted_accounts").all(), before);
    assert.equal(db.prepare("SELECT COUNT(*) AS count FROM hosted_accounts").get()?.count, 0);
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

test("durable legacy inventory records a pre-existing account without a jurisdiction", async () => {
  const { db, directory: accountDirectory } = await createTestDirectory();
  try {
    const entry = await accountDirectory.inventoryLegacy({
      subjectKey: "subject-legacy",
      bindingId: BINDING,
    });
    assert.deepEqual(entry, {
      subjectKey: "subject-legacy",
      bindingId: BINDING,
      jurisdiction: null,
      classification: "legacy-unrestricted",
    });
    assert.deepEqual(await accountDirectory.legacyInventory("subject-legacy"), entry);
    assert.equal(await accountDirectory.legacyInventory("unknown-subject"), undefined);
    assert.deepEqual(await accountDirectory.listLegacyInventory(), [entry]);
    // A legacy entry is not a validated row: no signup record exists.
    assert.equal(await accountDirectory.get("subject-legacy"), undefined);
  } finally {
    db.close();
  }
});

test("durable legacy inventory refuses any asserted jurisdiction, even on repeat", async () => {
  const { db, directory: accountDirectory } = await createTestDirectory();
  try {
    for (const jurisdiction of ["us", "eu", "apac"]) {
      await assert.rejects(
        accountDirectory.inventoryLegacy({
          subjectKey: "subject-legacy",
          bindingId: BINDING,
          jurisdiction,
        }),
        (error: unknown) =>
          error instanceof FailClosed && error.code === "legacy_jurisdiction_unverifiable",
      );
    }
    assert.deepEqual(await accountDirectory.listLegacyInventory(), []);
    // A relabel attempt against an already-inventoried subject still fails closed.
    await accountDirectory.inventoryLegacy({ subjectKey: "subject-legacy", bindingId: BINDING });
    await assert.rejects(
      accountDirectory.inventoryLegacy({
        subjectKey: "subject-legacy",
        bindingId: BINDING,
        jurisdiction: "us",
      }),
      (error: unknown) =>
        error instanceof FailClosed && error.code === "legacy_jurisdiction_unverifiable",
    );
    assert.deepEqual(await accountDirectory.listLegacyInventory(), [
      {
        subjectKey: "subject-legacy",
        bindingId: BINDING,
        jurisdiction: null,
        classification: "legacy-unrestricted",
      },
    ]);
  } finally {
    db.close();
  }
});

test("duplicate durable legacy inventory returns the preserved entry; conflicting binding fails closed", async () => {
  const { db, directory: accountDirectory } = await createTestDirectory();
  try {
    const first = await accountDirectory.inventoryLegacy({
      subjectKey: "subject-legacy",
      bindingId: BINDING,
    });
    const second = await accountDirectory.inventoryLegacy({
      subjectKey: "subject-legacy",
      bindingId: BINDING,
    });
    assert.deepEqual(second, first);
    await assert.rejects(
      accountDirectory.inventoryLegacy({
        subjectKey: "subject-legacy",
        bindingId: OTHER_BINDING,
      }),
      (error: unknown) =>
        error instanceof FailClosed && error.code === "duplicate_subject_conflict",
    );
    assert.deepEqual(await accountDirectory.listLegacyInventory(), [first]);
  } finally {
    db.close();
  }
});

test("durable signup for a legacy-inventoried subject fails closed without creating a validated row", async () => {
  const { db, directory: accountDirectory } = await createTestDirectory();
  try {
    await accountDirectory.inventoryLegacy({
      subjectKey: "subject-legacy",
      bindingId: BINDING,
    });
    await assert.rejects(
      accountDirectory.signup({
        subjectKey: "subject-legacy",
        bindingId: BINDING,
        jurisdiction: "us",
      }),
      (error: unknown) =>
        error instanceof FailClosed && error.code === "legacy_subject_requires_migration",
    );
    assert.equal(await accountDirectory.get("subject-legacy"), undefined);
    assert.equal(
      db.prepare("SELECT COUNT(*) AS count FROM hosted_accounts").get()?.count,
      0,
    );
  } finally {
    db.close();
  }
});

test("durable inventory of an already-registered subject fails closed", async () => {
  const { db, directory: accountDirectory } = await createTestDirectory();
  try {
    const record = await accountDirectory.signup({
      subjectKey: "subject-1",
      bindingId: BINDING,
      jurisdiction: "eu",
    });
    await assert.rejects(
      accountDirectory.inventoryLegacy({ subjectKey: "subject-1", bindingId: BINDING }),
      (error: unknown) =>
        error instanceof FailClosed && error.code === "legacy_subject_already_registered",
    );
    assert.deepEqual(
      await accountDirectory.signup({
        subjectKey: "subject-1",
        bindingId: BINDING,
        jurisdiction: "eu",
      }),
      record,
    );
    assert.deepEqual(await accountDirectory.listLegacyInventory(), []);
  } finally {
    db.close();
  }
});

test("durable legacy inventory lists every entry ordered by subject key", async () => {
  const { db, directory: accountDirectory } = await createTestDirectory();
  try {
    await accountDirectory.inventoryLegacy({ subjectKey: "subject-b", bindingId: BINDING });
    await accountDirectory.inventoryLegacy({ subjectKey: "subject-a", bindingId: OTHER_BINDING });
    assert.deepEqual(await accountDirectory.listLegacyInventory(), [
      {
        subjectKey: "subject-a",
        bindingId: OTHER_BINDING,
        jurisdiction: null,
        classification: "legacy-unrestricted",
      },
      {
        subjectKey: "subject-b",
        bindingId: BINDING,
        jurisdiction: null,
        classification: "legacy-unrestricted",
      },
    ]);
  } finally {
    db.close();
  }
});

test("legacy inventory CHECK constraints reject malformed identity and any jurisdiction label", async () => {
  const { db } = await createTestDirectory();
  const insert = db.prepare(
    "INSERT INTO legacy_hosted_accounts (subject_key, binding_id, classification) VALUES (?, ?, ?)",
  );

  try {
    assert.throws(() => insert.run("   ", BINDING, "legacy-unrestricted"), undefined, "blank subject");
    assert.throws(
      () => insert.run("subject", "0123456789abcdef0123456789abcde", "legacy-unrestricted"),
      undefined,
      "short binding id",
    );
    assert.throws(
      () => insert.run("subject", "g" + BINDING.slice(1), "legacy-unrestricted"),
      undefined,
      "non-hex binding id",
    );
    assert.throws(
      () => insert.run("subject", BINDING, "us"),
      undefined,
      "jurisdiction as classification",
    );
    assert.throws(
      () => insert.run("subject", BINDING, "other"),
      undefined,
      "unknown classification",
    );
    assert.equal(
      db.prepare("SELECT COUNT(*) AS count FROM legacy_hosted_accounts").get()?.count,
      0,
    );
  } finally {
    db.close();
  }
});

test("durable legacy reads fail closed on records outside the legacy contract", async () => {
  const { db, directory: accountDirectory } = await createTestDirectory();
  try {
    // Recreate the table without its CHECKs to plant rows the schema
    // could never have produced; the read path must still fail closed.
    db.exec("DROP TABLE legacy_hosted_accounts");
    db.exec(
      "CREATE TABLE legacy_hosted_accounts (subject_key TEXT PRIMARY KEY NOT NULL, binding_id TEXT NOT NULL UNIQUE, classification TEXT NOT NULL)",
    );
    const insert = db.prepare(
      "INSERT INTO legacy_hosted_accounts (subject_key, binding_id, classification) VALUES (?, ?, ?)",
    );
    insert.run("subject-odd", BINDING, "us");
    insert.run("subject-bad-binding", "google:sub", "legacy-unrestricted");
    await assert.rejects(
      accountDirectory.legacyInventory("subject-odd"),
      (error: unknown) =>
        error instanceof FailClosed && error.code === "invalid_legacy_inventory_record",
      "jurisdiction-labelled legacy row",
    );
    await assert.rejects(
      accountDirectory.legacyInventory("subject-bad-binding"),
      (error: unknown) =>
        error instanceof FailClosed && error.code === "invalid_legacy_inventory_record",
      "malformed binding id in legacy row",
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
