import { describe, test, expect, beforeAll, afterAll } from "bun:test";
import { createClient, type Client } from "@libsql/client";
import { mkdtempSync, rmSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { migrate } from "../src/store";
import { handleRequest, type ServerEnv } from "../src/server";

const __dirname = dirname(fileURLToPath(import.meta.url));
const FIXTURES = join(__dirname, "..", "fixtures");
const SECRET = "test-webhook-secret";
const env: ServerEnv = {
  GITHUB_WEBHOOK_SECRET: SECRET,
  PM_PROJECTOR_API_TOKEN: "test-api-token",
  PM_REPOS: "developing-today/code,developing-today/openchamber",
};

let db: Client;
let dir: string;

async function hmacSha256Hex(body: Uint8Array): Promise<string> {
  const key = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(SECRET),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const signature = new Uint8Array(
    await crypto.subtle.sign(
      "HMAC",
      key,
      body.buffer.slice(body.byteOffset, body.byteOffset + body.byteLength) as ArrayBuffer,
    ),
  );
  return Array.from(signature, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

async function sendFixture(file: string, event: string, deliveryId?: string): Promise<Response> {
  const body = readFileSync(join(FIXTURES, file), "utf8");
  const bytes = new TextEncoder().encode(body);
  const signature = await hmacSha256Hex(bytes);
  const headers = new Headers({
    "content-type": "application/json",
    "x-github-event": event,
    "x-hub-signature-256": `sha256=${signature}`,
  });
  if (deliveryId !== undefined) headers.set("x-github-delivery", deliveryId);

  return handleRequest(new Request("http://127.0.0.1/github/webhook", { method: "POST", headers, body }), db, env);
}

async function rowCount(table: "pmp_deliveries" | "pmp_items" | "pmp_push_events" | "pmp_comments") {
  const result = await db.execute(`SELECT COUNT(*) AS n FROM ${table}`);
  return Number((result.rows[0] as { n: number }).n);
}

async function expectNoAppliedOrRecordedData() {
  await expect(
    Promise.all([
      rowCount("pmp_deliveries"),
      rowCount("pmp_items"),
      rowCount("pmp_push_events"),
      rowCount("pmp_comments"),
    ]),
  ).resolves.toEqual([0, 0, 0, 0]);
}

beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), "pm-projector-delivery-id-"));
  db = createClient({ url: `file:${join(dir, "delivery-id.db")}` });
  await migrate(db);
});

afterAll(() => {
  db.close();
  rmSync(dir, { recursive: true, force: true });
});

describe("GitHub webhook delivery ID validation", () => {
  test("rejects distinct deliveries without the header before applying or recording either", async () => {
    const first = await sendFixture("01-push.json", "push");
    expect(first.status).toBe(400);
    expect(await first.json()).toEqual({ error: "missing_delivery_id" });
    await expectNoAppliedOrRecordedData();

    const second = await sendFixture("02-issue-opened.json", "issues");
    expect(second.status).toBe(400);
    expect(await second.json()).toEqual({ error: "missing_delivery_id" });
    await expectNoAppliedOrRecordedData();
  });

  test("rejects an empty delivery ID before applying or recording", async () => {
    const response = await sendFixture("01-push.json", "push", "");
    expect(response.status).toBe(400);
    expect(await response.json()).toEqual({ error: "missing_delivery_id" });
    await expectNoAppliedOrRecordedData();
  });

  test("preserves apply and same-GUID duplicate behavior", async () => {
    const first = await sendFixture("01-push.json", "push", "delivery-guid-1");
    expect(first.status).toBe(200);
    expect(await first.json()).toMatchObject({ status: "applied" });

    const duplicate = await sendFixture("01-push.json", "push", "delivery-guid-1");
    expect(duplicate.status).toBe(200);
    expect(await duplicate.json()).toEqual({ status: "duplicate" });
    await expect(Promise.all([rowCount("pmp_deliveries"), rowCount("pmp_push_events")])).resolves.toEqual([1, 1]);
  });
});
