import assert from "node:assert/strict";
import { test } from "node:test";
import { fileResponseForKey } from "../src/lib.ts";

test("fileResponseForKey rejects malformed percent-encoded keys with a safe 400", async () => {
  for (const encodedKey of ["%", "%zz", "uploads%2", "%E0%A4%A"]) {
    let lookedUp = false;
    const response = await fileResponseForKey(encodedKey, async () => {
      lookedUp = true;
      return null;
    });

    assert.equal(response.status, 400, encodedKey);
    assert.match(response.headers.get("content-type") ?? "", /application\/json/, encodedKey);
    assert.deepEqual(await response.json(), { error: "bad file key encoding" }, encodedKey);
    assert.equal(lookedUp, false, encodedKey);
  }
});

test("fileResponseForKey keeps valid encoded keys and downloads unchanged", async () => {
  let requestedKey = "";
  const response = await fileResponseForKey("uploads%2Freport.txt", async (key) => {
    requestedKey = key;
    return {
      body: new Response("file contents").body,
      httpMetadata: { contentType: "text/plain" },
    };
  });

  assert.equal(requestedKey, "uploads/report.txt");
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("content-type"), "text/plain");
  assert.equal(await response.text(), "file contents");
});

test("fileResponseForKey keeps the missing-object 404 response", async () => {
  const response = await fileResponseForKey("uploads%2Fmissing.txt", async (key) => {
    assert.equal(key, "uploads/missing.txt");
    return null;
  });

  assert.equal(response.status, 404);
  assert.deepEqual(await response.json(), { error: "not found in R2" });
});

test("fileResponseForKey leaves R2 failures for the worker's 500 handler", async () => {
  await assert.rejects(
    fileResponseForKey("uploads%2Freport.txt", async () => {
      throw new Error("R2 unavailable");
    }),
    /R2 unavailable/,
  );
});
