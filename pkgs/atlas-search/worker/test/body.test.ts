import { describe, expect, it } from "vitest";
import { MAX_SEARCH_BODY_BYTES, readTextUpTo } from "../src/body";

function counting(chunks: Uint8Array[]) {
  const seen = { bytes: 0 };
  let i = 0;
  const body = new ReadableStream<Uint8Array>(
    {
      pull(c) {
        if (i >= chunks.length) return c.close();
        const chunk = chunks[i++];
        seen.bytes += chunk.byteLength;
        c.enqueue(chunk);
      },
    },
    { highWaterMark: 0 },
  );
  return { body, seen };
}

describe("readTextUpTo (#11331)", () => {
  it("caps at 64 KiB by default", () => {
    expect(MAX_SEARCH_BODY_BYTES).toBe(64 * 1024);
  });

  it("refuses a declared oversized body without reading it", async () => {
    const { body, seen } = counting([new Uint8Array(64)]);
    const out = await readTextUpTo({ headers: new Headers({ "content-length": "64" }), body }, 10);
    expect(out).toBeNull();
    expect(seen.bytes).toBe(0);
  });

  it("stops reading an undeclared oversized body at the cap plus one chunk", async () => {
    const chunk = new Uint8Array(4);
    const { body, seen } = counting(Array.from({ length: 1000 }, () => chunk));
    const out = await readTextUpTo({ headers: new Headers(), body }, 10);
    expect(out).toBeNull();
    expect(seen.bytes).toBeLessThanOrEqual(10 + 2 * chunk.byteLength);
  });

  it("returns a body at or under the cap unchanged, multibyte included", async () => {
    const text = '{"a":"é😀"}';
    const bytes = new TextEncoder().encode(text);
    const { body } = counting([bytes.slice(0, 7), bytes.slice(7)]);
    const out = await readTextUpTo(
      { headers: new Headers({ "content-length": String(bytes.byteLength) }), body },
      bytes.byteLength,
    );
    expect(out).toBe(text);
  });

  it("treats a missing body as empty", async () => {
    expect(await readTextUpTo({ headers: new Headers(), body: null })).toBe("");
  });
});
