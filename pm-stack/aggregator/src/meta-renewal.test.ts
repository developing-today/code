import { describe, expect, test } from "bun:test";
import { mkdtemp, readFile, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  credentialFromCookies,
  pickMetaCookies,
  renewMetaSession,
  type MetaBrowserCookie,
  type MetaCredential,
  type MetaRenewalDeps,
} from "./meta-renewal.ts";

const NOW = new Date("2026-11-13T10:00:00.000Z");
const FUTURE = new Date(NOW.getTime() + 96 * 3_600_000).toISOString();
const SOON = new Date(NOW.getTime() + 4 * 3_600_000).toISOString();

const paths = async () => {
  const dir = await mkdtemp(join(tmpdir(), "meta-renewal-"));
  return { statePath: join(dir, "meta-renewal.json"), credentialPath: join(dir, "meta-credential.json") };
};

const storedCred = (expiresAt: string | null = FUTURE): MetaCredential => ({
  cookieHeader: "session=stored-value",
  capturedAt: "2026-11-13T09:00:00.000Z",
  expiresAt,
  cookieNames: ["session"],
});

const browserCookies = (expiresSeconds = (NOW.getTime() + 96 * 3_600_000) / 1000): MetaBrowserCookie[] => [
  { name: "session", value: "fresh-value", domain: ".meta.ai", expires: expiresSeconds },
  { name: "other", value: "x", domain: ".example.com", expires: expiresSeconds },
];

const baseDeps = (
  over: Partial<MetaRenewalDeps> & Pick<MetaRenewalDeps, "statePath" | "credentialPath">,
): MetaRenewalDeps => ({
  now: () => NOW,
  readLatest: async () => ({ status: "ok", measuredAt: NOW.toISOString(), attemptedAt: NOW.toISOString() }),
  ...over,
});

describe("pickMetaCookies / credentialFromCookies", () => {
  test("keeps only meta.ai cookies and uses the earliest expiry as the horizon", () => {
    const picked = pickMetaCookies(browserCookies());
    expect(picked.map((c) => c.name)).toEqual(["session"]);
    const cred = credentialFromCookies(browserCookies(), NOW);
    expect(cred?.cookieHeader).toBe("session=fresh-value");
    expect(cred?.expiresAt).toBe(FUTURE);
  });

  test("no meta cookies → no credential", () => {
    expect(credentialFromCookies([{ name: "a", value: "b", domain: ".example.com" }], NOW)).toBeNull();
  });
});

describe("renewMetaSession — provider-owned credential (#6120)", () => {
  test("stored credential validates with a healthy horizon → valid, no capture", async () => {
    const { statePath, credentialPath } = await paths();
    let captures = 0;
    const state = await renewMetaSession(
      baseDeps({
        statePath,
        credentialPath,
        readCredential: async () => storedCred(),
        validateCredential: async () => true,
        getBrowserCookies: async () => {
          captures += 1;
          return browserCookies();
        },
      }),
    );
    expect(state.status).toBe("valid");
    expect(state.capture.attempted).toBe(false);
    expect(captures).toBe(0);
    expect(state.horizonHours).toBe(96);
    expect(state.alert).toBeNull();
  });

  test("stored credential inside the low horizon → cadence re-capture stores the fresh one", async () => {
    const { statePath, credentialPath } = await paths();
    const writtenBox: { c: MetaCredential | null } = { c: null };
    const state = await renewMetaSession(
      baseDeps({
        statePath,
        credentialPath,
        readCredential: async () => storedCred(SOON),
        validateCredential: async (c) => c.cookieHeader.includes("fresh") || c.cookieHeader.includes("stored"),
        getBrowserCookies: async () => browserCookies(),
        writeCredential: async (c) => {
          writtenBox.c = c;
        },
      }),
    );
    expect(state.status).toBe("renewed");
    expect(writtenBox.c?.cookieHeader).toBe("session=fresh-value");
    expect(state.sessionExpiresAt).toBe(FUTURE);
  });

  test("stored credential stopped validating → re-capture recovers (auth-failure renewal)", async () => {
    const { statePath, credentialPath } = await paths();
    const state = await renewMetaSession(
      baseDeps({
        statePath,
        credentialPath,
        readCredential: async () => storedCred(),
        validateCredential: async (c) => c.cookieHeader.includes("fresh"),
        getBrowserCookies: async () => browserCookies(),
      }),
    );
    expect(state.status).toBe("recovered");
    expect(state.alert).toBeNull();
    const onDisk = JSON.parse(await readFile(credentialPath, "utf8")) as MetaCredential;
    expect(onDisk.cookieHeader).toBe("session=fresh-value");
    // The provider-owned credential location is owner-only.
    expect((await stat(credentialPath)).mode & 0o777).toBe(0o600);
  });

  test("forced failure: capture does not validate → nothing stored, attention, push fired once per episode", async () => {
    const { statePath, credentialPath } = await paths();
    const pushes: string[] = [];
    const deps = baseDeps({
      statePath,
      credentialPath,
      readCredential: async () => storedCred(),
      validateCredential: async () => false,
      getBrowserCookies: async () => browserCookies(),
      notifyPush: async (notice) => {
        pushes.push(notice.title);
        return "sent:1/1";
      },
    });
    const first = await renewMetaSession(deps);
    expect(first.status).toBe("attention");
    expect(first.capture.result).toBe("validation-failed");
    expect(first.notification.attempted).toBe(true);
    expect(first.notification.result).toBe("sent:1/1");
    expect(pushes).toHaveLength(1);
    expect(first.alertSince).toBe(NOW.toISOString());
    // Validate-before-store: the invalid capture never reached disk.
    await expect(readFile(credentialPath, "utf8")).rejects.toThrow();
    // The state file never carries the credential value.
    const stateRaw = await readFile(statePath, "utf8");
    expect(stateRaw).not.toContain("fresh-value");
    expect(stateRaw).not.toContain("stored-value");

    // Second tick of the same episode: still attention, no second push.
    const second = await renewMetaSession(deps);
    expect(second.status).toBe("attention");
    expect(second.notification.attempted).toBe(false);
    expect(pushes).toHaveLength(1);
    expect(second.alertSince).toBe(NOW.toISOString());
  });

  test("no stored credential and no browser session → attention with the last-resort remedy named", async () => {
    const { statePath, credentialPath } = await paths();
    const state = await renewMetaSession(
      baseDeps({
        statePath,
        credentialPath,
        readCredential: async () => null,
        getBrowserCookies: async () => [{ name: "a", value: "b", domain: ".example.com" }],
      }),
    );
    expect(state.status).toBe("attention");
    expect(state.capture.result).toBe("no-browser-session");
    expect(state.alert).toContain("Last-resort fallback");
    expect(state.credential.present).toBe(false);
  });

  test("CDP unreachable on first capture → attention, capture=cdp-unreachable", async () => {
    const { statePath, credentialPath } = await paths();
    const state = await renewMetaSession(
      baseDeps({
        statePath,
        credentialPath,
        readCredential: async () => null,
        getBrowserCookies: async () => {
          throw new Error("connect ECONNREFUSED");
        },
      }),
    );
    expect(state.status).toBe("attention");
    expect(state.capture.result).toBe("cdp-unreachable");
  });
});
