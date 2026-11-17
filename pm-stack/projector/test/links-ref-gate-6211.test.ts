// POST /links ref gate alignment (#6211): the shape regex alone admitted
// refs whose number is not a safe integer (e.g. a 20-digit issue number) -
// exactly the values the aggregator's parseRef rejects (#4952). One such
// stored row then 500s /search and project pages at render. The write gate
// now applies the same rule as parseRef: owner/repo#N AND a safe-integer N.
//
// Run: bun test test/links-ref-gate-6211.test.ts
import { describe, test, expect, beforeAll, afterAll } from "bun:test";
import { createClient, type Client } from "@libsql/client";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { migrate } from "../src/store";
import { handleRequest, isStorableLinkRef, type ServerEnv } from "../src/server";

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

const subject = "kaneo:code/CODE-6211";

beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), "pm-projector-ref-gate-"));
  db = createClient({ url: `file:${join(dir, "links.db")}` });
  await migrate(db);
});

afterAll(() => {
  db.close();
  rmSync(dir, { recursive: true, force: true });
});

describe("isStorableLinkRef", () => {
  test("accepts ordinary and max-safe-integer refs", () => {
    expect(isStorableLinkRef("developing-today/code#123")).toBe(true);
    expect(isStorableLinkRef("developing-today/code#9007199254740991")).toBe(true);
  });

  test("rejects unsafe-integer numbers the shape regex admits", () => {
    expect(isStorableLinkRef("developing-today/code#99999999999999999999")).toBe(false);
    expect(isStorableLinkRef("developing-today/code#9007199254740992")).toBe(false);
  });

  test("rejects non-refs and non-strings", () => {
    expect(isStorableLinkRef("not-a-ref")).toBe(false);
    expect(isStorableLinkRef("developing-today/code#12e3")).toBe(false);
    expect(isStorableLinkRef(123)).toBe(false);
    expect(isStorableLinkRef(null)).toBe(false);
  });
});

describe("POST /links safe-integer gate", () => {
  test("unsafe-integer ref -> 400, nothing stored", async () => {
    const r = await post({ subject, ref: "developing-today/code#99999999999999999999" });
    expect(r.status).toBe(400);
    expect(await linkCount()).toBe(0);
  });

  test("max safe integer ref -> 201, stored", async () => {
    const r = await post({ subject, ref: "developing-today/code#9007199254740991" });
    expect(r.status).toBe(201);
    expect(await linkCount()).toBe(1);
  });

  test("ordinary ref -> 201, stored", async () => {
    const r = await post({ subject, ref: "developing-today/code#6211" });
    expect(r.status).toBe(201);
    expect(await linkCount()).toBe(2);
  });
});
