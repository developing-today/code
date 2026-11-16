// Write-path bounds (#10635).
//
// POST /links and POST /hot/checkpoints used bare req.json(): one caller
// could force the process to buffer an unbounded body before validation.
// Both now read through readBoundedBody at MAX_WRITE_BODY_BYTES (413 on
// overflow, then JSON.parse with the existing 400 invalid_json), and the
// stored/compared string fields are bounded before any store call.
//
// Run: bun test test/write-bounds.test.ts
import { describe, test, expect, beforeAll, afterAll } from "bun:test";
import { createClient, type Client } from "@libsql/client";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { migrate } from "../src/store";
import {
  handleRequest,
  MAX_WRITE_BODY_BYTES,
  MAX_LINK_NOTE_CHARS,
  MAX_LINK_SUBJECT_CHARS,
  MAX_CONSUMER_CHARS,
  type ServerEnv,
} from "../src/server";

const TOKEN = "test-api-token";
const env: ServerEnv = { PM_PROJECTOR_API_TOKEN: TOKEN };

let db: Client;
let dir: string;

function authed(path: string, raw: string, contentType = "application/json"): Request {
  return new Request(`http://127.0.0.1${path}`, {
    method: "POST",
    headers: { "content-type": contentType, authorization: `Bearer ${TOKEN}` },
    body: raw,
  });
}

async function linkCount(): Promise<number> {
  const rows = await db.execute("SELECT COUNT(*) AS n FROM pmp_links");
  return Number((rows.rows[0] as any).n);
}

beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), "pm-projector-write-bounds-"));
  db = createClient({ url: `file:${join(dir, "bounds.db")}` });
  await migrate(db);
});

afterAll(() => {
  db.close();
  rmSync(dir, { recursive: true, force: true });
});

describe("POST body bounds (#10635)", () => {
  test("POST /links over the cap is 413 and stores nothing", async () => {
    const big = "x".repeat(MAX_WRITE_BODY_BYTES + 1);
    const before = await linkCount();
    const res = await handleRequest(
      authed("/links", JSON.stringify({ subject: "s", ref: "developing-today/code#1", note: big })),
      db,
      env,
    );
    expect(res.status).toBe(413);
    expect(await res.json()).toEqual({ error: "body_too_large" });
    expect(await linkCount()).toBe(before);
  });

  test("POST /hot/checkpoints over the cap is 413", async () => {
    const big = "y".repeat(MAX_WRITE_BODY_BYTES + 1);
    const res = await handleRequest(
      authed("/hot/checkpoints", JSON.stringify({ consumer: big, cursor: 0, highWater: 0 })),
      db,
      env,
    );
    expect(res.status).toBe(413);
    expect(await res.json()).toEqual({ error: "body_too_large" });
  });

  test("oversized note/subject/consumer are 400 and store nothing", async () => {
    const before = await linkCount();
    const noteRes = await handleRequest(
      authed(
        "/links",
        JSON.stringify({
          subject: "kaneo:code/CODE-1",
          ref: "developing-today/code#1",
          note: "n".repeat(MAX_LINK_NOTE_CHARS + 1),
        }),
      ),
      db,
      env,
    );
    expect(noteRes.status).toBe(400);
    const subjectRes = await handleRequest(
      authed(
        "/links",
        JSON.stringify({ subject: "s".repeat(MAX_LINK_SUBJECT_CHARS + 1), ref: "developing-today/code#1" }),
      ),
      db,
      env,
    );
    expect(subjectRes.status).toBe(400);
    const consumerRes = await handleRequest(
      authed(
        "/hot/checkpoints",
        JSON.stringify({ consumer: "c".repeat(MAX_CONSUMER_CHARS + 1), cursor: 0, highWater: 0 }),
      ),
      db,
      env,
    );
    expect(consumerRes.status).toBe(400);
    expect(await linkCount()).toBe(before);
  });

  test("existing 400 invalid_json and 401 unauthorized behavior unchanged", async () => {
    const badJson = await handleRequest(authed("/links", "{not json"), db, env);
    expect(badJson.status).toBe(400);
    expect(await badJson.json()).toEqual({ error: "invalid_json" });
    const badJsonCp = await handleRequest(authed("/hot/checkpoints", "{not json"), db, env);
    expect(badJsonCp.status).toBe(400);
    const noAuth = await handleRequest(
      new Request("http://127.0.0.1/links", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ subject: "s", ref: "developing-today/code#1" }),
      }),
      db,
      env,
    );
    expect(noAuth.status).toBe(401);
    const noAuthCp = await handleRequest(
      new Request("http://127.0.0.1/hot/checkpoints", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ consumer: "c", cursor: 0, highWater: 0 }),
      }),
      db,
      env,
    );
    expect(noAuthCp.status).toBe(401);
  });
});
