import { describe, test, expect, beforeAll, afterAll } from "bun:test";
import { createClient, type Client } from "@libsql/client";
import { mkdtempSync, rmSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { migrate } from "../src/store";
import { handleRequest, type ServerEnv } from "../src/server";
import { type FleetOperationSink, type OperationEnvelope, type OperationReceipt } from "../src/operation-forwarder";

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

async function sendFixture(
  file: string,
  event: string,
  deliveryId?: string,
  requestEnv: ServerEnv = env,
): Promise<Response> {
  const body = readFileSync(join(FIXTURES, file), "utf8");
  const bytes = new TextEncoder().encode(body);
  const signature = await hmacSha256Hex(bytes);
  const headers = new Headers({
    "content-type": "application/json",
    "x-github-event": event,
    "x-hub-signature-256": `sha256=${signature}`,
  });
  if (deliveryId !== undefined) headers.set("x-github-delivery", deliveryId);

  return handleRequest(
    new Request("http://127.0.0.1/github/webhook", { method: "POST", headers, body }),
    db,
    requestEnv,
  );
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

// DSP-01 (#9445): the existing signed webhook intake fans out to one
// authorized Fleet command; authentication failures produce zero
// Fleet effects. Appended as its own describe so the delivery-ID
// guarantees above stay byte-for-byte the original tests.
describe("DSP-01 webhook auth fan-out", () => {
  class FanoutSink implements FleetOperationSink {
    envelopes: OperationEnvelope[] = [];
    private receipts = new Map<string, OperationReceipt>();

    async accept(envelope: OperationEnvelope): Promise<OperationReceipt> {
      const existing = this.receipts.get(envelope.idempotencyKey);
      if (existing) return { ...existing, duplicate: true };
      const receipt: OperationReceipt = {
        receiptId: `rcpt-${this.receipts.size + 1}`,
        idempotencyKey: envelope.idempotencyKey,
        duplicate: false,
      };
      this.receipts.set(envelope.idempotencyKey, receipt);
      this.envelopes.push(envelope);
      return receipt;
    }
  }

  async function sendCommand(
    sink: FanoutSink,
    opts: { deliveryId: string; signature: string | null; mutate?: (p: any) => void },
  ): Promise<Response> {
    const payload = JSON.parse(readFileSync(join(FIXTURES, "03-issue-comment.json"), "utf8"));
    payload.comment.id = 92001;
    payload.comment.body = "/fleet do-now";
    opts.mutate?.(payload);
    const bytes = new TextEncoder().encode(JSON.stringify(payload));
    const headers = new Headers({
      "content-type": "application/json",
      "x-github-event": "issue_comment",
      "x-github-delivery": opts.deliveryId,
    });
    if (opts.signature !== null) headers.set("x-hub-signature-256", opts.signature);
    return handleRequest(
      new Request("http://127.0.0.1/github/webhook", { method: "POST", headers, body: bytes }),
      db,
      env,
      { operationSink: sink },
    );
  }

  test("pmo_dsp_01_existing_webhook_auth_fans_out_to_one_authorized_command", async () => {
    const sink = new FanoutSink();
    const body = JSON.parse(readFileSync(join(FIXTURES, "03-issue-comment.json"), "utf8"));
    body.comment.id = 92001;
    body.comment.body = "/fleet do-now";
    const bytes = new TextEncoder().encode(JSON.stringify(body));
    const goodSignature = `sha256=${await hmacSha256Hex(bytes)}`;

    // A validly signed command event fans out to exactly one command.
    const ok = await sendCommand(sink, { deliveryId: "fanout-ok", signature: goodSignature });
    expect(ok.status).toBe(200);
    const okBody = await ok.json();
    expect(okBody.status).toBe("applied");
    expect(okBody.operation.receiptId).toBe("rcpt-1");
    expect(sink.envelopes.length).toBe(1);
    expect(sink.envelopes[0]).toMatchObject({
      actor: "dezren39",
      verb: "do-now",
      repo: "developing-today/code",
      issue: 754,
      commentId: 92001,
      deliveryId: "fanout-ok",
    });
    // The idempotency key is semantic: it names the comment + verb,
    // not the delivery GUID.
    expect(sink.envelopes[0].idempotencyKey).toBe("ghcmd:v1:developing-today/code#754:comment:92001:do-now");

    // Tampered signature: rejected before any Fleet effect.
    const tampered = await sendCommand(sink, {
      deliveryId: "fanout-tampered",
      signature: `sha256=${"00".repeat(32)}`,
    });
    expect(tampered.status).toBe(401);
    expect(await tampered.json()).toEqual({ error: "invalid_signature" });
    expect(sink.envelopes.length).toBe(1);

    // Missing signature: rejected before any Fleet effect.
    const missing = await sendCommand(sink, { deliveryId: "fanout-missing", signature: null });
    expect(missing.status).toBe(401);
    expect(await missing.json()).toEqual({ error: "invalid_signature" });
    expect(sink.envelopes.length).toBe(1);

    // Oversized body (over the 10 MB intake bound): rejected before
    // any Fleet effect, even with a signature valid for that body.
    const bigPayload = JSON.parse(readFileSync(join(FIXTURES, "03-issue-comment.json"), "utf8"));
    bigPayload.comment.id = 92001;
    bigPayload.comment.body = `/fleet do-now ${"x".repeat(10_000_001)}`;
    const bigBytes = new TextEncoder().encode(JSON.stringify(bigPayload));
    const bigSignature = `sha256=${await hmacSha256Hex(bigBytes)}`;
    const tooLarge = await handleRequest(
      new Request("http://127.0.0.1/github/webhook", {
        method: "POST",
        headers: {
          "content-type": "application/json",
          "x-github-event": "issue_comment",
          "x-github-delivery": "fanout-oversized-signed",
          "x-hub-signature-256": bigSignature,
        },
        body: bigBytes,
      }),
      db,
      env,
      { operationSink: sink },
    );
    expect(tooLarge.status).toBe(413);
    expect(await tooLarge.json()).toEqual({ error: "body_too_large" });
    expect(sink.envelopes.length).toBe(1);
  });
});
