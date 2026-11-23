import { describe, expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import { parsePmMetadata, PM_METADATA_STORE_DDL } from "../src/metadata-store.ts";

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
    const result = parsePmMetadata({ pm_meta: 1, direct_parent: false, provenance: { lane: 7, future_flag: true }, links: ["bad-link"] });
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
      expect(new Set(indexes.map((index) => index.name))).toEqual(new Set([
        "pm_metadata_parent_idx", "pm_metadata_session_idx", "pm_metadata_lane_idx",
        "pm_metadata_host_idx", "pm_metadata_queue_idx", "sqlite_autoindex_pm_metadata_items_1",
      ]));
      const linkIndexes = db.query("PRAGMA index_list(pm_metadata_links)").all() as { name: string }[];
      expect(new Set(linkIndexes.map((index) => index.name))).toEqual(new Set([
        "pm_metadata_link_status_idx", "pm_metadata_link_target_idx",
        "sqlite_autoindex_pm_metadata_links_1",
      ]));
    } finally {
      db.close();
    }
  });
});
