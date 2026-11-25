/**
 * Browser session primitive for SSO relying-party slice H3 (#14261).
 *
 * Oracle: design §4.2 says the cookie is an httpOnly, Secure, SameSite=Lax
 * pointer with 12-hour idle expiry, and each request re-resolves live roles.
 * These tests fail if a provider bearer token is placed in the cookie, a
 * missing/expired/revoked session is accepted, or role revocation waits for
 * cookie expiry.
 */
import { describe, expect, test } from "bun:test";
import {
  authenticateBrowserSession,
  createBrowserSession,
  logoutBrowserSession,
  type BrowserSession,
  type BrowserSessionStore,
} from "../src/session.ts";
import type { ResolvedPrincipal } from "../src/front-door.ts";

const NOW = 1_800_000_000_000;
const PRINCIPAL: ResolvedPrincipal = {
  accountId: "acct-1",
  bindingId: "bind-1",
  roles: ["owner"],
  providerSub: "github|user-1",
  tokenId: "provider-jti-1",
};

class MemorySessions implements BrowserSessionStore {
  readonly sessions = new Map<string, BrowserSession>();
  async get(id: string) { return this.sessions.get(id) ?? null; }
  async put(session: BrowserSession) { this.sessions.set(session.id, session); }
  async touch(id: string, lastSeenAt: number) {
    const current = this.sessions.get(id);
    if (current) this.sessions.set(id, { ...current, lastSeenAt });
  }
  async delete(id: string) { this.sessions.delete(id); }
}

function request(cookie?: string): Request {
  return new Request("https://pm.example.test/", {
    headers: cookie ? { cookie } : {},
  });
}

describe("browser sessions (H3)", () => {
  test("issues an opaque secure cookie and re-resolves live roles on every request", async () => {
    const store = new MemorySessions();
    const setCookie = await createBrowserSession(PRINCIPAL, { store, now: () => NOW });
    expect(setCookie).toContain("HttpOnly");
    expect(setCookie).toContain("Secure");
    expect(setCookie).toContain("SameSite=Lax");
    expect(setCookie).toContain("Path=/");
    expect(setCookie).toContain("Max-Age=43200");
    expect(setCookie).not.toContain("provider-jti-1");
    expect(setCookie).not.toContain("github|user-1");

    const cookie = setCookie.split(";")[0]!;
    let roleLookups = 0;
    const auth = await authenticateBrowserSession(request(cookie), {
      store,
      now: () => NOW + 1_000,
      resolveLivePrincipal: async (session) => {
        roleLookups += 1;
        expect(session.principal.accountId).toBe("acct-1");
        return { ...PRINCIPAL, roles: ["read-only"] };
      },
    });
    expect(auth).toEqual({ ok: true, principal: { ...PRINCIPAL, roles: ["read-only"] } });
    expect(roleLookups).toBe(1);
  });

  test("rejects missing, idle-expired, and revoked sessions", async () => {
    const store = new MemorySessions();
    expect(await authenticateBrowserSession(request(), {
      store,
      now: () => NOW,
      resolveLivePrincipal: async () => PRINCIPAL,
    })).toEqual({ ok: false, reason: "missing_session" });

    const setCookie = await createBrowserSession(PRINCIPAL, { store, now: () => NOW });
    const cookie = setCookie.split(";")[0]!;
    expect(await authenticateBrowserSession(request(cookie), {
      store,
      now: () => NOW + 12 * 60 * 60 * 1000,
      resolveLivePrincipal: async () => PRINCIPAL,
    })).toEqual({ ok: false, reason: "expired_session" });

    const freshCookie = await createBrowserSession(PRINCIPAL, { store, now: () => NOW });
    expect(await authenticateBrowserSession(request(freshCookie.split(";")[0]!), {
      store,
      now: () => NOW + 1_000,
      resolveLivePrincipal: async () => null,
    })).toEqual({ ok: false, reason: "revoked_session" });
  });

  test("logout deletes the server-side session and expires the cookie", async () => {
    const store = new MemorySessions();
    const setCookie = await createBrowserSession(PRINCIPAL, { store, now: () => NOW });
    const cookie = setCookie.split(";")[0]!;
    const cleared = await logoutBrowserSession(request(cookie), {
      store,
      now: () => NOW + 1_000,
    });
    expect(cleared).toContain("Max-Age=0");
    expect(cleared).toContain("HttpOnly");
    expect(cleared).toContain("Secure");
    expect(store.sessions.size).toBe(0);
  });
});
