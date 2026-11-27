import { describe, expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import {
  createPmMetadataStore,
  getPmMetadata,
  parsePmMetadata,
  PM_METADATA_STORE_DDL,
  upsertPmMetadata,
} from "../src/metadata-store.ts";

describe("PM Metadata schema v1 adapter", () => {
  test("keeps the complete section-12 object and extracts registered query fields", () => {
    const source = {
      pm_meta: 1,
      direct_parent: { repo: "developing-today/code", number: 3447, kind: "epic" },
      links: [{ target: { repo: "developing-today/code", number: 123 }, relation: "parent", status: "historical" }],
      provenance: { queue_id: "q-7", session_id: "ses_A9", lane: "oc-code-03", host: "build-host" },
      new_classifier_data: { score: 0.75 },
    };

    const result = parsePmMetadata(source);
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.value.metadata).toEqual(source);
    expect(result.value.schemaVersion).toBe(1);
    expect(result.value.indexes).toEqual({
      directParentRepo: "developing-today/code",
      directParentNumber: 3447,
      sessionId: "ses_A9",
      lane: "oc-code-03",
      host: "build-host",
      queueId: "q-7",
      links: [{ targetRepo: "developing-today/code", targetNumber: 123, relation: "parent", status: "historical" }],
    });
    expect(result.value.extras).toEqual({ new_classifier_data: { score: 0.75 } });
  });

  test("preserves optional absence and relocates wrong-typed registered values to extras", () => {
    const result = parsePmMetadata({
      pm_meta: 1,
      direct_parent: false,
      provenance: { lane: 7, future_flag: true },
      links: ["bad-link"],
    });
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.value.indexes.directParentRepo).toBeUndefined();
    expect(result.value.indexes.sessionId).toBeUndefined();
    expect(result.value.extras).toEqual({
      direct_parent: false,
      "provenance.lane": 7,
      "provenance.future_flag": true,
      "links.0": "bad-link",
    });
    expect(result.value.metadata).toEqual({
      pm_meta: 1,
      direct_parent: false,
      provenance: { lane: 7, future_flag: true },
      links: ["bad-link"],
    });
  });

  test("rejects unknown versions rather than silently extracting with the v1 adapter", () => {
    expect(parsePmMetadata({ pm_meta: 2 })).toMatchObject({ ok: false });
    expect(parsePmMetadata({ direct_parent: null })).toMatchObject({ ok: false });
  });

  test("creates versioned source rows and indexed link/query fields", () => {
    const db = new Database(":memory:");
    try {
      for (const statement of PM_METADATA_STORE_DDL) db.exec(statement);
      const columns = db.query("PRAGMA table_info(pm_metadata_items)").all() as { name: string }[];
      const names = new Set(columns.map((column) => column.name));
      expect(names.has("artifact_id")).toBe(true);
      expect(names.has("source_url")).toBe(true);
      expect(names.has("schema_version")).toBe(true);
      expect(names.has("metadata_json")).toBe(true);
      expect(names.has("extras_json")).toBe(true);
      const indexes = db.query("PRAGMA index_list(pm_metadata_items)").all() as { name: string }[];
      expect(new Set(indexes.map((index) => index.name))).toEqual(
        new Set([
          "pm_metadata_parent_idx",
          "pm_metadata_session_idx",
          "pm_metadata_lane_idx",
          "pm_metadata_host_idx",
          "pm_metadata_queue_idx",
          "sqlite_autoindex_pm_metadata_items_1",
        ]),
      );
      const linkIndexes = db.query("PRAGMA index_list(pm_metadata_links)").all() as { name: string }[];
      expect(new Set(linkIndexes.map((index) => index.name))).toEqual(
        new Set(["pm_metadata_link_status_idx", "pm_metadata_link_target_idx", "sqlite_autoindex_pm_metadata_links_1"]),
      );
    } finally {
      db.close();
    }
  });

  test("upserts and reads a versioned record idempotently by artifact identity", () => {
    const db = new Database(":memory:");
    try {
      createPmMetadataStore(db);
      const source = {
        pm_meta: 1,
        direct_parent: { repo: "developing-today/code", number: 3447, kind: "epic" },
        links: [{ target: { repo: "developing-today/code", number: 123 }, relation: "parent", status: "historical" }],
        provenance: { queue_id: "q-7", session_id: "ses_A9", lane: "oc-code-03", host: "build-host" },
        extras: { classifier: { score: 0.75 } },
      };
      const parsed = parsePmMetadata(source);
      expect(parsed.ok).toBe(true);
      if (!parsed.ok) return;
      const record = {
        artifactId: "sha256:sample-metadata-1",
        repo: "developing-today/code",
        itemNumber: 9074,
        itemKind: "issue",
        sourceUrl: "https://github.com/developing-today/code/issues/9074",
        parsed: parsed.value,
        sourceUpdatedAt: "2026-11-23T03:00:00Z",
      };

      // Oracle: artifact identity is the primary key. Replaying the same
      // source replaces one row and its derived links; it never creates a
      // second row or duplicate link, and the read reconstructs the parsed
      // source exactly enough for the query projection.
      upsertPmMetadata(db, record);
      upsertPmMetadata(db, record);

      expect(db.query("SELECT COUNT(*) AS count FROM pm_metadata_items").get()).toEqual({ count: 1 });
      expect(db.query("SELECT COUNT(*) AS count FROM pm_metadata_links").get()).toEqual({ count: 1 });
      expect(getPmMetadata(db, record.artifactId)).toMatchObject({
        artifactId: record.artifactId,
        repo: record.repo,
        itemNumber: record.itemNumber,
        itemKind: record.itemKind,
        sourceUrl: record.sourceUrl,
        schemaVersion: 1,
        metadata: source,
        indexes: parsed.value.indexes,
      });

      const changedSource = { ...source, links: [] };
      const changed = parsePmMetadata(changedSource);
      expect(changed.ok).toBe(true);
      if (!changed.ok) return;
      upsertPmMetadata(db, { ...record, parsed: changed.value, sourceUpdatedAt: "2026-11-23T04:00:00Z" });
      expect(db.query("SELECT COUNT(*) AS count FROM pm_metadata_items").get()).toEqual({ count: 1 });
      expect(db.query("SELECT COUNT(*) AS count FROM pm_metadata_links").get()).toEqual({ count: 0 });
      expect(getPmMetadata(db, record.artifactId)?.indexes.links).toEqual([]);
    } finally {
      db.close();
    }
  });

  test("unchanged artifact replays issue no row writes and one changed link updates only its row", () => {
    const db = new Database(":memory:");
    try {
      createPmMetadataStore(db);
      const source = {
        pm_meta: 1,
        links: [
          { target: { repo: "developing-today/code", number: 123 }, relation: "parent", status: "active" },
          { target: { repo: "developing-today/code", number: 456 }, relation: "blocks", status: "active" },
        ],
      };
      const parsed = parsePmMetadata(source);
      expect(parsed.ok).toBe(true);
      if (!parsed.ok) return;
      const record = {
        artifactId: "sha256:metadata-write-audit",
        repo: "developing-today/code",
        itemNumber: 14865,
        itemKind: "issue",
        sourceUrl: "https://github.com/developing-today/code/issues/14865",
        parsed: parsed.value,
        sourceUpdatedAt: "2026-11-24T00:00:00Z",
      };
      upsertPmMetadata(db, record);

      // Oracle: SQL triggers record each persisted row mutation. An exact
      // replay must mutate zero rows; changing one derived link may update
      // only its source row and that one indexed link row.
      db.exec(`
        CREATE TABLE write_audit (table_name TEXT, operation TEXT);
        CREATE TRIGGER audit_item_update AFTER UPDATE ON pm_metadata_items
          BEGIN INSERT INTO write_audit VALUES ('items', 'UPDATE'); END;
        CREATE TRIGGER audit_link_update AFTER UPDATE ON pm_metadata_links
          BEGIN INSERT INTO write_audit VALUES ('links', 'UPDATE'); END;
        CREATE TRIGGER audit_link_delete AFTER DELETE ON pm_metadata_links
          BEGIN INSERT INTO write_audit VALUES ('links', 'DELETE'); END;
        CREATE TRIGGER audit_link_insert AFTER INSERT ON pm_metadata_links
          BEGIN INSERT INTO write_audit VALUES ('links', 'INSERT'); END;
      `);

      upsertPmMetadata(db, record);
      expect(db.query("SELECT table_name, operation FROM write_audit ORDER BY table_name, operation").all()).toEqual([]);

      const changedSource = {
        ...source,
        links: source.links.map((link, index) => index === 1 ? { ...link, status: "historical" } : link),
      };
      const changed = parsePmMetadata(changedSource);
      expect(changed.ok).toBe(true);
      if (!changed.ok) return;
      upsertPmMetadata(db, { ...record, parsed: changed.value, sourceUpdatedAt: "2026-11-24T01:00:00Z" });
      expect(db.query("SELECT table_name, operation FROM write_audit ORDER BY table_name, operation").all()).toEqual([
        { table_name: "items", operation: "UPDATE" },
        { table_name: "links", operation: "UPDATE" },
      ]);
      expect(getPmMetadata(db, record.artifactId)?.indexes.links).toEqual(changed.value.indexes.links);
    } finally {
      db.close();
    }
  });
});
