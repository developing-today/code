// Oracle for #15793 criterion 1 (parent #13494): the mcpx Worker
// answers unauthenticated GET /version with the commit it was
// deployed from, in the `version`-field shape the edge probe reads
// (commitOf in pm-stack/console/edge/src/worker-checks.ts) — "unknown",
// never a fabricated commit, when the deploy was unstamped — and the
// rest of the route surface is unchanged.
import { describe, expect, it, vi } from "vitest";
import worker, { versionAnswer, type Env } from "../src/index";
import { MemoryKV } from "./memory-kv";

const baseEnv: Env = {
  BRIDGE_BASE: "https://mcpx-link.security.cab",
  ACCESS_CLIENT_ID: "bridge-id.access",
  ACCESS_CLIENT_SECRET: "bridge-secret",
  MACHINE_BEARER: "machine-bearer-for-tests",
  ACCESS_AUD: "authorize-app-aud",
  ACCESS_TEAM_DOMAIN: "team.example.com",
  OAUTH_KV: new MemoryKV() as unknown as KVNamespace,
};

const req = (path: string, method = "GET") => new Request(`https://mcpx.security.cab${path}`, { method });

describe("GET /version (#15793 criterion 1)", () => {
  it("stamped: names the deployed commit from the VERSION var, without touching the bridge", async () => {
    const bridge = vi.fn();
    vi.stubGlobal("fetch", bridge);
    const res = await worker.fetch(req("/version"), { ...baseEnv, VERSION: "abc123" });
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true, service: "mcpx", version: "abc123" });
    expect(bridge).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });

  it("unstamped: answers unknown, never a fabricated commit", async () => {
    const res = await worker.fetch(req("/version"), baseEnv);
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true, service: "mcpx", version: "unknown" });
  });

  it("versionAnswer trims the stamp and falls back to unknown", () => {
    expect(versionAnswer({ ...baseEnv, VERSION: "  def456 " })).toEqual({
      ok: true,
      service: "mcpx",
      version: "def456",
    });
    expect(versionAnswer(baseEnv).version).toBe("unknown");
    expect(versionAnswer({ ...baseEnv, VERSION: "   " }).version).toBe("unknown");
  });

  it("POST /version is refused 405 (the route is GET-only)", async () => {
    const res = await worker.fetch(req("/version", "POST"), { ...baseEnv, VERSION: "abc123" });
    expect(res.status).toBe(405);
  });

  it("/healthz is unchanged", async () => {
    const res = await worker.fetch(req("/healthz"), { ...baseEnv, VERSION: "abc123" });
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true, service: "mcpx" });
  });
});
