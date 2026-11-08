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

async function sendFixture(file: string, event: string, deliveryId?: string, requestEnv: ServerEnv = env): Promise<Response> {
  const body = readFileSync(join(FIXTURES, file), "utf8");
  const bytes = new TextEncoder().encode(body);
  const signature = await hmacSha256Hex(bytes);
  const headers = new Headers({
    "content-type": "application/json",
    "x-github-event": event,
    "x-hub-signature-256": `sha256=${signature}`,
  });
  if (deliveryId !== undefined) headers.set("x-github-delivery", deliveryId);

  return handleRequest(new Request("http://127.0.0.1/github/webhook", { method: "POST", headers, body }), db, requestEnv);
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
  test("rejects an invalid signature without publishing a freshness invalidation", async () => {
    let freshnessAttempts = 0;
    const sink = Bun.serve({
      hostname: "127.0.0.1",
      port: 0,
      fetch: async () => {
        freshnessAttempts += 1;
        return new Response(null, { status: 202 });
      },
    });
    try {
      const body = readFileSync(join(FIXTURES, "02-issue-opened.json"), "utf8");
      const response = await handleRequest(
        new Request("http://127.0.0.1/github/webhook", {
          method: "POST",
          headers: {
            "content-type": "application/json",
            "x-github-event": "issues",
            "x-github-delivery": "invalid-signature-delivery",
            "x-hub-signature-256": `sha256=${"00".repeat(32)}`,
          },
          body,
        }),
        db,
        { ...env, PM_AGG_FRESHNESS_URL: `http://127.0.0.1:${sink.port}/publish` },
      );

      expect(response.status).toBe(401);
      expect(await response.json()).toEqual({ error: "invalid_signature" });
      expect(freshnessAttempts).toBe(0);
      await expectNoAppliedOrRecordedData();
    } finally {
      sink.stop(true);
    }
  });

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

  test("retries source acceptance when the freshness journal is unavailable", async () => {
    let attempts = 0;
    const inputs: unknown[] = [];
    const sink = Bun.serve({
      hostname: "127.0.0.1",
      port: 0,
      fetch: async (request) => {
        attempts += 1;
        inputs.push(await request.json());
        return new Response(null, { status: attempts === 1 ? 503 : 202 });
      },
    });
    const sinkEnv = { ...env, PM_AGG_FRESHNESS_URL: `http://127.0.0.1:${sink.port}/publish` };
    try {
      const failed = await sendFixture("02-issue-opened.json", "issues", "delivery-freshness-retry", sinkEnv);
      expect(failed.status).toBe(500);
      expect(await failed.json()).toEqual({ error: "apply_failed" });

      const retried = await sendFixture("02-issue-opened.json", "issues", "delivery-freshness-retry", sinkEnv);
      expect(retried.status).toBe(200);
      expect(await retried.json()).toMatchObject({ status: "applied" });
      expect(attempts).toBe(2);
      expect(inputs[0]).toEqual(inputs[1]);
      expect(inputs[0]).toMatchObject({ eventId: "github:delivery-freshness-retry", source: "github" });
    } finally {
      sink.stop(true);
    }
  });
});
