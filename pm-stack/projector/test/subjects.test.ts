// Regression tests for #2693 — projector /subjects/ decode guard.
//
// The /subjects/<segment> branch passed its path segment through an
// unguarded decodeURIComponent, so a malformed percent-encoding
// (%E0%A4%A, %zz, a trailing %) threw URIError out of handleRequest
// and the request died as a non-JSON 500. The fix guards just the
// decode: the URIError becomes a JSON 400. These tests pin the
// malformed forms alongside the well-formed controls (existing
// subject -> 200, unknown subject -> 404 subject_not_found).
//
// Run: bun test test/subjects.test.ts
import { describe, test, expect, beforeAll, afterAll } from "bun:test";
import { createClient, type Client } from "@libsql/client";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { migrate, upsertLink } from "../src/store";
import { handleRequest, type ServerEnv } from "../src/server";

const env: ServerEnv = {};
let db: Client;
let dir: string;

function get(path: string): Promise<Response> {
  return handleRequest(new Request(`http://127.0.0.1${path}`), db, env);
}

beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), "projector-subjects-"));
  db = createClient({ url: `file:${join(dir, "subjects.db")}` });
  await migrate(db);
  await upsertLink(db, {
    subject: "gh:owner/repo#1",
    ref: "owner/repo#1",
    relation: "relates",
  });
});

afterAll(() => {
  db.close();
  rmSync(dir, { recursive: true, force: true });
});

describe("GET /subjects/<malformed percent-encoding>", () => {
  for (const bad of ["%E0%A4%A", "%zz", "trailing%", "%2"]) {
    test(`${bad} returns a JSON 400 and never throws`, async () => {
      const res = await get(`/subjects/${bad}`);
      expect(res.status).toBe(400);
      expect(await res.json()).toEqual({
        error: "malformed percent-encoding in subject",
      });
    });
  }
});

describe("GET /subjects/<well-formed> controls", () => {
  test("existing subject returns 200 with its view", async () => {
    const res = await get(`/subjects/${encodeURIComponent("gh:owner/repo#1")}`);
    expect(res.status).toBe(200);
    const view = (await res.json()) as { subject: string };
    expect(view.subject).toBe("gh:owner/repo#1");
  });

  test("unknown subject returns 404 subject_not_found", async () => {
    const res = await get("/subjects/plain");
    expect(res.status).toBe(404);
    expect(await res.json()).toEqual({ error: "subject_not_found" });
  });

  test("well-formed encoding of an unknown subject 404s cleanly", async () => {
    const res = await get("/subjects/valid%20name");
    expect(res.status).toBe(404);
    expect(await res.json()).toEqual({ error: "subject_not_found" });
  });
});
