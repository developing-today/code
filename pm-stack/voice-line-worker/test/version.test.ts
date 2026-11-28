import { describe, expect, test } from "bun:test";
import worker from "../src/index";
import { versionAnswer } from "../src/version";

// Oracle: #15793 criterion 1 — every in-service Worker names the commit
// it was deployed from on unauthenticated GET /version, in the shape
// the edge probe reads (commitOf in
// pm-stack/console/edge/src/worker-checks.ts takes the `version` field).

const req = (path: string, method = "GET") => new Request(`https://voice-line.test${path}`, { method });

describe("versionAnswer", () => {
  test("names the stamped commit", () => {
    expect(versionAnswer({ VERSION: "abc123" })).toEqual({ ok: true, service: "voice-line", version: "abc123" });
  });

  test("is honest when unstamped", () => {
    expect(versionAnswer({})).toEqual({ ok: true, service: "voice-line", version: "unknown" });
  });
});

describe("GET /version through the fetch handler", () => {
  test("answers without auth and touches no binding", async () => {
    const res = await worker.fetch(req("/version"), { VERSION: "abc123" } as never);
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true, service: "voice-line", version: "abc123" });
  });

  test("answers unknown when unstamped", async () => {
    const res = await worker.fetch(req("/version"), {} as never);
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true, service: "voice-line", version: "unknown" });
  });

  test("POST /version is a 405", async () => {
    const res = await worker.fetch(req("/version", "POST"), { VERSION: "abc123" } as never);
    expect(res.status).toBe(405);
  });

  test("unknown paths still 404", async () => {
    const res = await worker.fetch(req("/nope"), {} as never);
    expect(res.status).toBe(404);
  });
});
