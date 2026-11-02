import { describe, expect, test } from "bun:test";
import { MAX_JSON_BODY_BYTES, readJsonBody } from "../src/request-body.ts";

function streamedRequest(
  chunks: Uint8Array[],
  options: { contentLength?: string; onPull?: () => void; onCancel?: () => void } = {},
): Request {
  const body = new ReadableStream<Uint8Array>(
    {
      pull(controller) {
        options.onPull?.();
        const next = chunks.shift();
        if (next) controller.enqueue(next);
        else controller.close();
      },
      cancel() {
        options.onCancel?.();
      },
    },
    { highWaterMark: 0 },
  );
  const headers = new Headers();
  if (options.contentLength !== undefined) headers.set("content-length", options.contentLength);
  return new Request("http://localhost/", {
    method: "POST",
    headers,
    body,
    duplex: "half",
  } as RequestInit);
}

describe("bounded JSON request body reader", () => {
  test("accepts a body whose UTF-8 byte length is exactly the limit", async () => {
    const value = "é".repeat((MAX_JSON_BODY_BYTES - 8) / 2);
    const text = `{"x":"${value}"}`;
    expect(new TextEncoder().encode(text).byteLength).toBe(MAX_JSON_BODY_BYTES);

    await expect(readJsonBody(new Request("http://localhost/", { method: "POST", body: text }))).resolves.toEqual({
      x: value,
    });
  });

  test("rejects a declared oversized body before reading it", async () => {
    let pulls = 0;
    let cancelled = false;
    const req = streamedRequest([new TextEncoder().encode("{}")], {
      contentLength: String(MAX_JSON_BODY_BYTES + 1),
      onPull: () => pulls++,
      onCancel: () => (cancelled = true),
    });

    await expect(readJsonBody(req)).rejects.toThrow("body too large");
    expect(pulls).toBe(0);
    expect(cancelled).toBe(true);
  });

  test("counts UTF-8 bytes rather than JavaScript characters", async () => {
    const text = `{"x":"${"é".repeat(MAX_JSON_BODY_BYTES / 2 - 3)}"}`;
    expect(text.length).toBeLessThan(MAX_JSON_BODY_BYTES);
    expect(new TextEncoder().encode(text).byteLength).toBeGreaterThan(MAX_JSON_BODY_BYTES);

    await expect(readJsonBody(new Request("http://localhost/", { method: "POST", body: text }))).rejects.toThrow(
      "body too large",
    );
  });

  test("cancels an oversized streamed body at the first chunk crossing the limit", async () => {
    let pulls = 0;
    let cancelled = false;
    const req = streamedRequest([new Uint8Array(60_000), new Uint8Array(40_001), new Uint8Array(1_000_000)], {
      onPull: () => pulls++,
      onCancel: () => (cancelled = true),
    });

    await expect(readJsonBody(req)).rejects.toThrow("body too large");
    expect(pulls).toBe(2);
    expect(cancelled).toBe(true);
  });
});
