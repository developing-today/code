import { describe, expect, test } from "bun:test";
import { startBrowserLogin, completeBrowserLogin, type PendingLogin, type PendingLoginStore } from "../src/oauth-pkce.ts";
import { handlePmServiceFetch } from "../src/worker.ts";
import type { BrowserOAuthDeps } from "../src/oauth-pkce.ts";
import type { BrowserSession, BrowserSessionStore } from "../src/session.ts";
import type { ResolvedPrincipal } from "../src/front-door.ts";

class Pending implements PendingLoginStore {
  value: PendingLogin | null = null;
  async put(value: PendingLogin) { this.value = value; }
  async get(state: string) { return this.value?.state === state ? this.value : null; }
  async delete(state: string) { if (this.value?.state === state) this.value = null; }
}
class Sessions implements BrowserSessionStore {
  values = new Map<string, BrowserSession>();
  async get(id: string) { return this.values.get(id) ?? null; }
  async put(value: BrowserSession) { this.values.set(value.id, value); }
  async touch(id: string, lastSeenAt: number) {
    const value = this.values.get(id);
    if (value) this.values.set(id, { ...value, lastSeenAt });
  }
  async delete(id: string) { this.values.delete(id); }
}

const NOW = 1_800_000_000_000;
const PRINCIPAL: ResolvedPrincipal = {
  accountId: "account-1", bindingId: "binding-1", roles: ["owner"], providerSub: "user-1", tokenId: "token-id-secret",
};
const bytes = (size: number) => new Uint8Array(size).fill(7);

function deps(overrides: Partial<BrowserOAuthDeps> = {}): BrowserOAuthDeps {
  return {
    provider: {
      authorizationEndpoint: "https://id.example.test/authorize",
      clientId: "pm-service",
      redirectUri: "https://pm.example.test/oauth/callback",
      exchangeCode: async () => { throw new Error("unexpected token exchange"); },
    },
    pending: new Pending(), sessions: new Sessions(),
    resolveLivePrincipal: async () => PRINCIPAL,
    frontDoor: {
      issuer: "https://id.example.test", audience: "pm-service", jwks: { keys: [] },
      resolveAccountId: async () => "account-1", loadRoles: async () => ["owner"],
    },
    now: () => NOW, randomBytes: bytes, ...overrides,
  };
}

describe("PM worker browser OAuth (H3)", () => {
  test("login starts S256 PKCE with state and nonce in a secure HttpOnly cookie", async () => {
    const config = deps();
    const response = await startBrowserLogin(config);
    const target = new URL(response.headers.get("location")!);
    expect(response.status).toBe(302);
    expect(target.searchParams.get("response_type")).toBe("code");
    expect(target.searchParams.get("code_challenge_method")).toBe("S256");
    expect(target.searchParams.get("code_challenge")).toBeTruthy();
    expect(target.searchParams.get("state")).toBeTruthy();
    expect(target.searchParams.get("nonce")).toBeTruthy();
    expect(response.headers.get("set-cookie")).toContain("HttpOnly; Secure; SameSite=Lax");
    expect(config.pending.value?.verifier).toBeTruthy();
    expect(target.searchParams.get("code_challenge")).not.toBe(config.pending.value?.verifier);
  });

  test("callback validates nonce and issues only an opaque browser session cookie", async () => {
    const pair = await crypto.subtle.generateKey(
      { name: "RSASSA-PKCS1-v1_5", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" },
      true, ["sign", "verify"],
    );
    const jwk = await crypto.subtle.exportKey("jwk", pair.publicKey);
    let config: BrowserOAuthDeps;
    let expectedNonce = "";
    let expectedVerifier = "";
    config = deps({ frontDoor: {
      issuer: "https://id.example.test", audience: "pm-service",
      jwks: { keys: [{ ...jwk, kid: "fixture", alg: "RS256", use: "sig" }] },
      now: () => NOW / 1000,
      resolveAccountId: async () => "account-1", loadRoles: async () => ["owner"],
    }, provider: {
      authorizationEndpoint: "https://id.example.test/authorize", clientId: "pm-service",
      redirectUri: "https://pm.example.test/oauth/callback",
      exchangeCode: async ({ verifier }) => {
        expect(verifier).toBe(expectedVerifier);
        const header = btoa(JSON.stringify({ alg: "RS256", typ: "JWT", kid: "fixture" })).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
        const payload = btoa(JSON.stringify({
          iss: "https://id.example.test", aud: "pm-service", sub: "user-1", exp: NOW / 1000 + 60,
          iat: NOW / 1000, jti: "provider-token-secret", binding_id: "binding-1", roles: ["owner"],
          nonce: expectedNonce,
        })).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
        const input = `${header}.${payload}`;
        const signature = await crypto.subtle.sign("RSASSA-PKCS1-v1_5", pair.privateKey, new TextEncoder().encode(input));
        const encoded = btoa(String.fromCharCode(...new Uint8Array(signature))).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
        return { idToken: `${input}.${encoded}` };
      },
    } });
    const start = await startBrowserLogin(config);
    const authorization = new URL(start.headers.get("location")!);
    const state = authorization.searchParams.get("state")!;
    expectedNonce = authorization.searchParams.get("nonce")!;
    expectedVerifier = config.pending.value!.verifier;
    const callback = await completeBrowserLogin(new Request(`https://pm.example.test/oauth/callback?code=one-time-code&state=${state}`, {
      headers: { cookie: `__Host-pm_oauth_state=${state}` },
    }), config);
    expect(callback.status).toBe(303);
    expect(callback.headers.get("location")).toBe("/");
    expect(callback.headers.get("set-cookie")).toContain("__Host-pm_session=");
    expect(callback.headers.get("set-cookie")).not.toContain("provider-token-secret");
    expect(config.sessions.values.size).toBe(1);
  });

  test("callback rejects missing/mismatched state before provider exchange", async () => {
    let exchanges = 0;
    const config = deps({ provider: {
      authorizationEndpoint: "https://id.example.test/authorize", clientId: "pm-service",
      redirectUri: "https://pm.example.test/oauth/callback",
      exchangeCode: async () => { exchanges += 1; return { idToken: "secret" }; },
    } });
    const response = await completeBrowserLogin(new Request("https://pm.example.test/oauth/callback?code=abc&state=wrong", {
      headers: { cookie: "__Host-pm_oauth_state=other" },
    }), config);
    expect(response.status).toBe(303);
    expect(response.headers.get("location")).toContain("invalid_state");
    expect(response.headers.get("set-cookie")).toContain("Max-Age=0");
    expect(exchanges).toBe(0);
    expect(response.headers.get("location")).not.toContain("secret");
  });

  test("worker rejects unauthenticated requests when browser auth is configured", async () => {
    const config = deps();
    const response = await handlePmServiceFetch(new Request("https://pm.example.test/"), { BROWSER_OAUTH: config });
    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "unauthenticated" });
  });
});
