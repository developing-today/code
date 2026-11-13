import { fetchMock } from "cloudflare:test";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { Turso } from "../src/turso";

// Response-envelope contract for the Turso pipeline client (#6949):
// a 200 whose body is not the pipeline shape must fail as
// "turso pipeline failed: …", never resolve as a successful write and
// never escape as a raw TypeError/SyntaxError.
const DB = "https://turso-shape.test";

beforeAll(() => {
  fetchMock.activate();
  fetchMock.disableNetConnect();
});
afterEach(() => fetchMock.assertNoPendingInterceptors());

function replyOnce(body: string, contentType = "application/json") {
  fetchMock
    .get(DB)
    .intercept({ path: "/v2/pipeline", method: "POST" })
    .reply(200, body, { headers: { "content-type": contentType } });
}

const client = () => new Turso(DB, "test-token");

const okEnvelope = JSON.stringify({
  results: [{ type: "ok", response: { result: { rows: [[{ type: "text", value: "main" }]] } } }, { type: "ok" }],
});

describe("Turso pipeline response envelope", () => {
  it("resolves a well-formed envelope", async () => {
    replyOnce(okEnvelope);
    await expect(client().queryText("SELECT default_branch FROM t")).resolves.toBe("main");
  });

  it("keeps the per-statement error contract", async () => {
    replyOnce(JSON.stringify({ results: [{ type: "error", error: { message: "no such table" } }] }));
    await expect(client().execute("SELECT 1")).rejects.toThrow("turso pipeline failed: no such table");
  });

  it("rejects a null body as malformed, not a TypeError", async () => {
    replyOnce("null");
    await expect(client().execute("INSERT INTO ci_events VALUES (1)")).rejects.toThrow(
      "turso pipeline failed: malformed response (missing results)",
    );
  });

  it("rejects a results-less object body instead of reading the write as success", async () => {
    replyOnce("{}");
    await expect(client().execute("INSERT INTO ci_events VALUES (1)")).rejects.toThrow(
      "turso pipeline failed: malformed response (missing results)",
    );
  });

  it("rejects a scalar body and a queryText on a results-less body", async () => {
    replyOnce('"ok"');
    await expect(client().execute("INSERT INTO ci_events VALUES (1)")).rejects.toThrow(
      "turso pipeline failed: malformed response (missing results)",
    );
    replyOnce("{}");
    await expect(client().queryText("SELECT default_branch FROM t")).rejects.toThrow(
      "turso pipeline failed: malformed response (missing results)",
    );
  });

  it("rejects a non-JSON body in contract form, not a raw SyntaxError", async () => {
    replyOnce("<html>bad gateway</html>", "text/html");
    await expect(client().execute("SELECT 1")).rejects.toThrow("turso pipeline failed: malformed response (not JSON)");
  });

  it("rejects an envelope whose results lack the statement result", async () => {
    replyOnce(JSON.stringify({ results: [] }));
    await expect(client().execute("SELECT 1")).rejects.toThrow(
      "turso pipeline failed: malformed response (missing statement result)",
    );
    replyOnce(JSON.stringify({ results: [null, { type: "ok" }] }));
    await expect(client().execute("SELECT 1")).rejects.toThrow(
      "turso pipeline failed: malformed response (missing statement result)",
    );
  });
});
