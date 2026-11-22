// #10504 reader flip: the collector reads the orchestration section
// from the latest orch_runs.document; orchestration.json is the
// fallback + rollback path. The oracle for every case is the #10504
// fallback contract: on DB failure serve the last good document and
// surface the failure — never a partial document.
import { describe, expect, test } from "bun:test";
import {
  LATEST_ORCH_RUN_SQL,
  orchestrationDocumentFromRows,
  orchestrationSourceFromEnv,
  resolveOrchestrationDocument,
} from "../src/orchestration-db.ts";

const doc = { runId: "orch-1", version: 1 };
const validate = (d: unknown): string | null =>
  typeof d === "object" && d !== null && "runId" in d ? null : "missing runId";

describe("orchestrationDocumentFromRows", () => {
  test("extracts runId + document from a well-shaped row", () => {
    expect(orchestrationDocumentFromRows([{ run_id: "orch-1", document: doc }])).toEqual({
      runId: "orch-1",
      document: doc,
    });
  });

  test("rejects shapes that would serve a partial document", () => {
    expect(orchestrationDocumentFromRows([])).toBeNull();
    expect(orchestrationDocumentFromRows(null)).toBeNull();
    expect(orchestrationDocumentFromRows([{ run_id: "orch-1" }])).toBeNull();
    expect(orchestrationDocumentFromRows([{ run_id: "orch-1", document: null }])).toBeNull();
    expect(orchestrationDocumentFromRows([{ run_id: "orch-1", document: [1] }])).toBeNull();
    expect(orchestrationDocumentFromRows([{ document: doc }])).toBeNull();
    expect(orchestrationDocumentFromRows([{ run_id: 7, document: doc }])).toBeNull();
  });

  test("the SQL reads the newest run's complete document", () => {
    expect(LATEST_ORCH_RUN_SQL).toContain("FROM public.orch_runs");
    expect(LATEST_ORCH_RUN_SQL).toContain("document");
    expect(LATEST_ORCH_RUN_SQL).toContain("ORDER BY generated_at DESC");
    expect(LATEST_ORCH_RUN_SQL).toContain("LIMIT 1");
  });
});

describe("orchestrationSourceFromEnv", () => {
  test("db is the default; only the exact file value rolls back", () => {
    expect(orchestrationSourceFromEnv({})).toBe("db");
    expect(orchestrationSourceFromEnv({ ORCHESTRATION_SOURCE: "db" })).toBe("db");
    expect(orchestrationSourceFromEnv({ ORCHESTRATION_SOURCE: "garbage" })).toBe("db");
    expect(orchestrationSourceFromEnv({ ORCHESTRATION_SOURCE: "file" })).toBe("file");
    expect(orchestrationSourceFromEnv({ ORCHESTRATION_SOURCE: " File " })).toBe("file");
  });
});

describe("resolveOrchestrationDocument", () => {
  test("db success serves the DB document and never reads the file", async () => {
    let fileReads = 0;
    const res = await resolveOrchestrationDocument({
      source: "db",
      readDb: async () => doc,
      readFile: async () => {
        fileReads++;
        return { runId: "orch-file" };
      },
      validate,
    });
    expect(res).toEqual({ document: doc, source: "db" });
    expect(fileReads).toBe(0);
  });

  test("db failure serves the last good file document and surfaces the failure", async () => {
    const fileDoc = { runId: "orch-file" };
    const res = await resolveOrchestrationDocument({
      source: "db",
      readDb: async () => {
        throw new Error("Turso store query failed (HTTP 503)");
      },
      readFile: async () => fileDoc,
      validate,
    });
    expect(res.source).toBe("file");
    expect(res.document).toBe(fileDoc);
    expect(res.dbError).toContain("HTTP 503");
  });

  test("an invalid DB document is a DB failure, never served", async () => {
    const fileDoc = { runId: "orch-file" };
    const res = await resolveOrchestrationDocument({
      source: "db",
      readDb: async () => ({ partial: true }),
      readFile: async () => fileDoc,
      validate,
    });
    expect(res.source).toBe("file");
    expect(res.document).toBe(fileDoc);
    expect(res.dbError).toContain("structural check");
    expect(res.dbError).toContain("missing runId");
  });

  test("db failure + missing file reports both, with fileMissing set", async () => {
    const enoent = Object.assign(new Error("ENOENT: no such file"), { code: "ENOENT" });
    const res = await resolveOrchestrationDocument({
      source: "db",
      readDb: async () => {
        throw new Error("no Turso store token available (bws)");
      },
      readFile: async () => {
        throw enoent;
      },
      validate,
    });
    expect(res.document).toBeNull();
    expect(res.source).toBeNull();
    expect(res.dbError).toContain("store token");
    expect(res.fileMissing).toBe(true);
  });

  test("rollback (source=file) never consults the DB", async () => {
    let dbReads = 0;
    const fileDoc = { runId: "orch-file" };
    const res = await resolveOrchestrationDocument({
      source: "file",
      readDb: async () => {
        dbReads++;
        return doc;
      },
      readFile: async () => fileDoc,
      validate,
    });
    expect(res).toEqual({ document: fileDoc, source: "file" });
    expect(dbReads).toBe(0);
  });

  test("rollback + missing file is the pre-#899 skipped state, no dbError", async () => {
    const enoent = Object.assign(new Error("ENOENT: no such file"), { code: "ENOENT" });
    const res = await resolveOrchestrationDocument({
      source: "file",
      readDb: async () => doc,
      readFile: async () => {
        throw enoent;
      },
      validate,
    });
    expect(res.document).toBeNull();
    expect(res.fileMissing).toBe(true);
    expect(res.dbError).toBeUndefined();
  });
});
