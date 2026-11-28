// Oracle for #15793 criterion 1 (parent #13494): the pm-notify Worker
// answers unauthenticated GET /version with the commit it was deployed
// from, in the `version`-field shape the edge probe reads (commitOf in
// pm-stack/console/edge/src/worker-checks.ts) — "unknown", never a
// fabricated commit, when the deploy was unstamped — and the closed
// route list still 404s everything else.
import { describe, expect, test } from "bun:test";
import worker, { versionAnswer } from "../src/index.ts";

const req = (path: string, method = "GET") => new Request(`https://pm-notify.example${path}`, { method });

// A stand-in Durable Object namespace: /version is answered before the
// stub is ever called, but the handler still builds the stub first for
// every other route, so the fake only needs to exist, not behave.
const fakeEnv = {
  PM_NOTIFY: {
    idFromName: (name: string) => name,
    get: () => ({ fetch: async () => new Response("not found", { status: 404 }) }),
  },
};

describe("GET /version (#15793 criterion 1)", () => {
  test("stamped: names the deployed commit from the VERSION var", async () => {
    const res = await worker.fetch(req("/version"), { VERSION: "abc123" } as never);
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true, service: "pm-notify", version: "abc123" });
  });

  test("unstamped: answers unknown, never a fabricated commit", async () => {
    const res = await worker.fetch(req("/version"), {} as never);
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true, service: "pm-notify", version: "unknown" });
  });

  test("versionAnswer trims the stamp and falls back to unknown", () => {
    expect(versionAnswer({ VERSION: "  def456 " } as never)).toEqual({
      ok: true,
      service: "pm-notify",
      version: "def456",
    });
    expect(versionAnswer({} as never).version).toBe("unknown");
    expect(versionAnswer({ VERSION: "   " } as never).version).toBe("unknown");
  });

  test("the closed route list still 404s unknown routes", async () => {
    const res = await worker.fetch(req("/nope"), fakeEnv as never);
    expect(res.status).toBe(404);
  });

  test("POST /version is not a route (closed list 404s it)", async () => {
    const res = await worker.fetch(req("/version", "POST"), fakeEnv as never);
    expect(res.status).toBe(404);
  });
});
