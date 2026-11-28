// Byte-budgeted JSON body reader tests (#16519): the shared reader
// every ingress handler now parses through refuses oversized bodies
// with 413 before parsing, and keeps the doors' long-standing 400
// invalid_body refusal for unparseable bodies under the cap.

import assert from "node:assert/strict";
import { test } from "node:test";
import { MAX_JSON_BODY_BYTES, readBodyText, readJsonBody } from "../src/body.ts";
import { FailClosed } from "../src/errors.ts";

function post(body: BodyInit): Request {
  return new Request("https://fleet-hosted.test/v1/queue/health", { method: "POST", body });
}

function isFailClosed(status: number, code: string) {
  return (err: unknown) => err instanceof FailClosed && err.status === status && err.code === code;
}

test("a JSON body under the cap parses", async () => {
  assert.deepEqual(await readJsonBody(post(JSON.stringify({ ok: true, n: 3 }))), { ok: true, n: 3 });
});

test("an unparseable body under the cap is a 400 invalid_body", async () => {
  await assert.rejects(readJsonBody(post("not json")), isFailClosed(400, "invalid_body"));
});

test("a body over the cap is refused 413 by its declared content-length", async () => {
  const big = JSON.stringify({ blob: "x".repeat(MAX_JSON_BODY_BYTES) });
  await assert.rejects(readJsonBody(post(big)), isFailClosed(413, "body_too_large"));
});

test("an oversized body is refused 413 before parsing, even when it is not JSON", async () => {
  // Not valid JSON at any prefix: a parse-first reader would answer
  // 400; the cap must win, proving the refusal happens pre-parse.
  await assert.rejects(readJsonBody(post("x".repeat(64)), 16), isFailClosed(413, "body_too_large"));
});

test("a streamed body with no content-length is refused 413 as soon as it passes the cap", async () => {
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(new TextEncoder().encode('{"blob":"'));
      controller.enqueue(new TextEncoder().encode("x".repeat(64)));
      controller.enqueue(new TextEncoder().encode('"}'));
      controller.close();
    },
  });
  const req = new Request("https://fleet-hosted.test/v1/kv/v2/pipeline", {
    method: "POST",
    body: stream,
    // @ts-expect-error duplex is required by undici for stream bodies and absent from the TS lib type
    duplex: "half",
  });
  assert.equal(req.headers.get("content-length"), null);
  await assert.rejects(readBodyText(req, 16), isFailClosed(413, "body_too_large"));
});

test("readBodyText returns an under-cap body verbatim", async () => {
  assert.equal(await readBodyText(post("hello"), 16), "hello");
});
