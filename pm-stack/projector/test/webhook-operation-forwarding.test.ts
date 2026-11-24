// DSP-01 (#9445): signed webhook intake fans out to canonical Fleet
// command acceptance, and projection delivery dedupe must never
// suppress that forward. The Fleet authority is an injected sink in
// these tests; its contract (authorization + durable acceptance keyed
// by the semantic command id, not the delivery GUID) is pinned on the
// Rust side by pmo_dsp_01_command_acceptance_is_durable_separate_from_projection_delivery.
import { describe, test, expect, beforeAll, afterAll } from "bun:test";
import { createClient, type Client } from "@libsql/client";
import { mkdtempSync, rmSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { migrate } from "../src/store";
import { handleRequest, type ServerEnv } from "../src/server";
import {
  parseFleetCommand,
  type FleetOperationSink,
  type OperationEnvelope,
  type OperationReceipt,
} from "../src/operation-forwarder";

const __dirname = dirname(fileURLToPath(import.meta.url));
const FIXTURES = join(__dirname, "..", "fixtures");
const SECRET = "test-webhook-secret";
const env: ServerEnv = {
  GITHUB_WEBHOOK_SECRET: "test-webhook-secret",
  PM_PROJECTOR_API_TOKEN: "test-api-token",
  PM_REPOS: "developing-today/code,developing-today/openchamber",
};

let db: Client;
let dir: string;

/** In-memory Fleet authority: dedupes by semantic idempotency key and
 *  returns the stored receipt on repeats, exactly as the canonical
 *  acceptance contract requires. `failNext` simulates the authority
 *  being unavailable for the first N accepts. */
class FakeFleetSink implements FleetOperationSink {
  receipts = new Map<string, OperationReceipt>();
  effects: OperationEnvelope[] = [];
  calls = 0;
  failNext = 0;

  async accept(envelope: OperationEnvelope): Promise<OperationReceipt> {
    this.calls += 1;
    if (this.failNext > 0) {
      this.failNext -= 1;
      throw new Error("fleet_unavailable");
    }
    const existing = this.receipts.get(envelope.idempotencyKey);
    if (existing) return { ...existing, duplicate: true };
    const receipt: OperationReceipt = {
      receiptId: `rcpt-${this.receipts.size + 1}`,
      idempotencyKey: envelope.idempotencyKey,
      duplicate: false,
    };
    this.receipts.set(envelope.idempotencyKey, receipt);
    this.effects.push(envelope);
    return receipt;
  }
}

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

async function sendComment(
  input: { deliveryId: string; commentId: number; body: string },
  sink?: FakeFleetSink,
): Promise<Response> {
  const payload = JSON.parse(readFileSync(join(FIXTURES, "03-issue-comment.json"), "utf8"));
  payload.comment.id = input.commentId;
  payload.comment.body = input.body;
  const bytes = new TextEncoder().encode(JSON.stringify(payload));
  const signature = await hmacSha256Hex(bytes);
  return handleRequest(
    new Request("http://127.0.0.1/github/webhook", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "x-github-event": "issue_comment",
        "x-github-delivery": input.deliveryId,
        "x-hub-signature-256": `sha256=${signature}`,
      },
      body: bytes,
    }),
    db,
    env,
    sink ? { operationSink: sink } : {},
  );
}

async function deliveryOutcome(deliveryId: string): Promise<string | null> {
  const result = await db.execute({
    sql: "SELECT outcome FROM pmp_deliveries WHERE delivery_id = ?",
    args: [deliveryId],
  });
  return result.rows.length ? String((result.rows[0] as unknown as { outcome: string }).outcome) : null;
}

beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), "pm-projector-op-forward-"));
  db = createClient({ url: `file:${join(dir, "op-forward.db")}` });
  await migrate(db);
});

afterAll(() => {
  db.close();
  rmSync(dir, { recursive: true, force: true });
});

describe("webhook operation forwarding (DSP-01)", () => {
  test("operation_forward_retries_after_projector_delivery_is_applied", async () => {
    const sink = new FakeFleetSink();
    sink.failNext = 1; // first Fleet acceptance attempt fails

    // First delivery: projection applies, Fleet acceptance fails.
    const first = await sendComment({ deliveryId: "op-delivery-1", commentId: 91001, body: "/fleet do-now" }, sink);
    expect(first.status).toBe(500);
    expect(await first.json()).toEqual({ error: "operation_forward_failed" });
    // The projection WAS applied and recorded — this is the state the
    // old delivery shortcut would have hidden behind "duplicate".
    expect(await deliveryOutcome("op-delivery-1")).toBe("applied");
    expect(sink.effects.length).toBe(0);

    // Redelivery of the same GUID: the already-applied branch must
    // retry the forward instead of suppressing it.
    const second = await sendComment({ deliveryId: "op-delivery-1", commentId: 91001, body: "/fleet do-now" }, sink);
    expect(second.status).toBe(200);
    const secondBody = await second.json();
    expect(secondBody.status).toBe("duplicate");
    expect(secondBody.operation.receiptId).toBe("rcpt-1");
    expect(secondBody.operation.duplicate).toBe(false);
    expect(sink.effects.length).toBe(1);
    expect(sink.effects[0].verb).toBe("do-now");
    expect(sink.effects[0].actor).toBe("dezren39");
    expect(sink.effects[0].repo).toBe("developing-today/code");
    expect(sink.effects[0].issue).toBe(754);

    // A third delivery: exactly one command effect ever; the stored
    // Fleet receipt comes back marked as a duplicate.
    const third = await sendComment({ deliveryId: "op-delivery-1", commentId: 91001, body: "/fleet do-now" }, sink);
    expect(third.status).toBe(200);
    const thirdBody = await third.json();
    expect(thirdBody.operation.receiptId).toBe("rcpt-1");
    expect(thirdBody.operation.duplicate).toBe(true);
    expect(sink.effects.length).toBe(1);

    // A *different* delivery GUID carrying the same comment is a new
    // projection delivery but the same semantic command: still one
    // effect, same stored receipt.
    const other = await sendComment({ deliveryId: "op-delivery-2", commentId: 91001, body: "/fleet do-now" }, sink);
    expect(other.status).toBe(200);
    const otherBody = await other.json();
    expect(otherBody.operation.receiptId).toBe("rcpt-1");
    expect(otherBody.operation.duplicate).toBe(true);
    expect(sink.effects.length).toBe(1);
  });

  test("non-command comments project without forwarding", async () => {
    const sink = new FakeFleetSink();
    const response = await sendComment(
      { deliveryId: "op-delivery-plain", commentId: 91002, body: "Reproducing with the projector fixture." },
      sink,
    );
    expect(response.status).toBe(200);
    expect((await response.json()).status).toBe("applied");
    expect(sink.calls).toBe(0);
  });

  test("without a sink, webhook behavior is projection-only and unchanged", async () => {
    const first = await sendComment({ deliveryId: "op-delivery-nosink", commentId: 91003, body: "/fleet do-now" });
    expect(first.status).toBe(200);
    expect((await first.json()).status).toBe("applied");

    const duplicate = await sendComment({
      deliveryId: "op-delivery-nosink",
      commentId: 91003,
      body: "/fleet do-now",
    });
    expect(duplicate.status).toBe(200);
    expect(await duplicate.json()).toEqual({ status: "duplicate" });
  });

  test("parseFleetCommand recognizes only a leading /fleet command", () => {
    expect(parseFleetCommand("/fleet do-now")).toEqual({ verb: "do-now", args: "" });
    expect(parseFleetCommand("/fleet priority high")).toEqual({ verb: "priority", args: "high" });
    expect(parseFleetCommand("/fleet agent status")).toEqual({ verb: "agent-status", args: "" });
    expect(parseFleetCommand("  /fleet park  ")).toEqual({ verb: "park", args: "" });
    expect(parseFleetCommand("please run /fleet do-now")).toBeNull();
    expect(parseFleetCommand("/fleet frobnicate")).toBeNull();
    expect(parseFleetCommand("/fleet")).toBeNull();
    expect(parseFleetCommand("ordinary comment")).toBeNull();
  });
});
