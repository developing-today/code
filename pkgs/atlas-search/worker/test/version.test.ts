import { SELF } from "cloudflare:test";
import { describe, expect, it } from "vitest";
import worker, { type Env } from "../src/index";
import { versionAnswer } from "../src/version";

// The /version route reads only env.VERSION, ahead of every gated
// route, so a minimal env exercises it through the fetch handler.
function envWith(version?: string): Env {
  return { VERSION: version } as unknown as Env;
}

describe("GET /version (#15793 criterion 1)", () => {
  // Oracle: #15793 criterion 1 (parent #13494) — every in-service Worker
  // answers unauthenticated GET /version with the commit it was deployed
  // from, in the `version`-field shape the edge probe reads (commitOf in
  // pm-stack/console/edge/src/worker-checks.ts); "unknown", never a
  // fabricated commit, when the deploy was unstamped.
  it("names the stamped deployed commit, unauthenticated", async () => {
    const res = await worker.fetch(new Request("https://worker.test/version"), envWith("abc123def456"));
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true, service: "atlas-search", version: "abc123def456" });
  });

  it("answers unknown when unstamped or blank, and trims the stamp", async () => {
    const unstamped = await worker.fetch(new Request("https://worker.test/version"), envWith());
    expect(await unstamped.json()).toEqual({ ok: true, service: "atlas-search", version: "unknown" });
    expect(versionAnswer({ VERSION: "  abc123  " })).toEqual({ ok: true, service: "atlas-search", version: "abc123" });
    expect(versionAnswer({ VERSION: "   " }).version).toBe("unknown");
    expect(versionAnswer({}).version).toBe("unknown");
  });

  it("is a 405 for non-GET", async () => {
    const res = await worker.fetch(new Request("https://worker.test/version", { method: "POST" }), envWith("abc123"));
    expect(res.status).toBe(405);
  });

  it("the configured worker answers unknown until a stamped deploy", async () => {
    const live = await SELF.fetch("https://worker.test/version");
    expect(live.status).toBe(200);
    expect(await live.json()).toEqual({ ok: true, service: "atlas-search", version: "unknown" });
  });
});
