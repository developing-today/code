// Whole-body validation for POST /links (#2974): the branch used to
// validate only subject/ref — relation was String()-coerced into the
// store ('a,b' / '[object Object]') while the 201 echoed the raw value,
// a non-string note/createdBy threw the SQLite bind error out of
// handleRequest uncaught, a numeric note was stored as 42.0, and an
// empty-string subject was accepted. Every violation must now be a
// JSON 400 that stores nothing, and a store failure a JSON 500.
//
// Run: bun test test/links-validation.test.ts
import { describe, test, expect, beforeAll, afterAll } from "bun:test";
import { createClient, type Client } from "@libsql/client";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { migrate } from "../src/store";
import { handleRequest, type ServerEnv } from "../src/server";

const TOKEN = "test-api-token";
const env: ServerEnv = { PM_PROJECTOR_API_TOKEN: TOKEN };

let db: Client;
let dir: string;

async function post(body: unknown): Promise<{ status: number; body: any }> {
  const res = await handleRequest(
    new Request("http://127.0.0.1/links", {
      method: "POST",
      headers: { "content-type": "application/json", authorization: `Bearer ${TOKEN}` },
      body: JSON.stringify(body),
    }),
    db,
    env,
  );
  return { status: res.status, body: await res.json() };
}

async function linkCount(): Promise<number> {
  const rows = await db.execute("SELECT COUNT(*) AS n FROM pmp_links");
  return Number((rows.rows[0] as any).n);
}

const base = { subject: "kaneo:code/CODE-1", ref: "developing-today/code#2974" };

beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), "pm-projector-links-"));
  db = createClient({ url: `file:${join(dir, "links.db")}` });
  await migrate(db);
});

afterAll(() => {
  db.close();
  rmSync(dir, { recursive: true, force: true });
});

describe("POST /links whole-body validation", () => {
  test("array relation -> 400, nothing stored", async () => {
    const r = await post({ ...base, relation: ["a", "b"] });
    expect(r.status).toBe(400);
    expect(await linkCount()).toBe(0);
  });

  test("object relation -> 400, nothing stored", async () => {
    const r = await post({ ...base, relation: { x: 1 } });
    expect(r.status).toBe(400);
    expect(await linkCount()).toBe(0);
  });

  test("object note -> 400 (no uncaught store throw), nothing stored", async () => {
    const r = await post({ ...base, note: { text: "hi" } });
    expect(r.status).toBe(400);
    expect(await linkCount()).toBe(0);
  });

  test("array createdBy -> 400, nothing stored", async () => {
    const r = await post({ ...base, createdBy: ["drew"] });
    expect(r.status).toBe(400);
    expect(await linkCount()).toBe(0);
  });

  test("numeric note -> 400, nothing stored", async () => {
    const r = await post({ ...base, note: 42 });
    expect(r.status).toBe(400);
    expect(await linkCount()).toBe(0);
  });

  test("empty-string subject -> 400, nothing stored", async () => {
    const r = await post({ ...base, subject: "" });
    expect(r.status).toBe(400);
    expect(await linkCount()).toBe(0);
  });

  test("valid control -> 201 and verbatim round-trip", async () => {
    const r = await post({ ...base, relation: "implements", note: "a note", createdBy: "drew" });
    expect(r.status).toBe(201);
    expect(r.body.relation).toBe("implements");
    const rows = await db.execute("SELECT subject, ref, relation, note, created_by FROM pmp_links");
    expect(rows.rows.length).toBe(1);
    const row = rows.rows[0] as any;
    expect(row.subject).toBe(base.subject);
    expect(row.ref).toBe(base.ref);
    expect(row.relation).toBe("implements");
    expect(row.note).toBe("a note");
    expect(row.created_by).toBe("drew");
  });

  test("absent relation defaults to 'relates'; null note/createdBy accepted", async () => {
    const r = await post({ subject: "kaneo:code/CODE-2", ref: "developing-today/code#2974", note: null, createdBy: null });
    expect(r.status).toBe(201);
    expect(r.body.relation).toBe("relates");
  });
});
