import { describe, expect, test } from "bun:test";
import { mkdtemp, readFile, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  cdpSetCookieParam,
  credentialFromCookies,
  pickMetaCookies,
  renewMetaSession,
  restoreBackoffMs,
  STALE_MS,
  restoreCookiesFromCredential,
  type MetaBrowserCookie,
  type MetaCredential,
  type MetaExporterDoc,
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

describe("renewMetaSession — browser-jar restore (#10839)", () => {
  const okDoc: MetaExporterDoc = { status: "ok", measuredAt: NOW.toISOString(), attemptedAt: NOW.toISOString() };
  const expiredDoc: MetaExporterDoc = { status: "session-expired", measuredAt: null, attemptedAt: NOW.toISOString() };

  test("restoreCookiesFromCredential: captured cookies win; legacy header parses onto .meta.ai", () => {
    const withCookies: MetaCredential = { ...storedCred(), cookies: browserCookies() };
    const fromArray = restoreCookiesFromCredential(withCookies);
    expect(fromArray.map((c) => c.name)).toEqual(["session"]);
    expect(fromArray[0]?.domain).toBe(".meta.ai");

    const fromHeader = restoreCookiesFromCredential(storedCred());
    expect(fromHeader).toEqual([
      { name: "session", value: "stored-value", domain: ".meta.ai", secure: true, expires: Date.parse(FUTURE) / 1000 },
    ]);

    expect(restoreBackoffMs(1)).toBe(30 * 60 * 1000);
    expect(restoreBackoffMs(2)).toBe(2 * 3_600_000);
    expect(restoreBackoffMs(3)).toBe(6 * 3_600_000);
    expect(restoreBackoffMs(9)).toBe(6 * 3_600_000);
  });

  test("cdpSetCookieParam: a restore writes the captured attributes back, not a bare name=value", () => {
    expect(
      cdpSetCookieParam({
        name: "session",
        value: "v",
        domain: ".meta.ai",
        path: "/api",
        expires: 1_900_000_000,
        secure: true,
        httpOnly: true,
        sameSite: "None",
      }),
    ).toEqual({
      name: "session",
      value: "v",
      domain: ".meta.ai",
      path: "/api",
      expires: 1_900_000_000,
      secure: true,
      httpOnly: true,
      sameSite: "None",
    });
    // Nothing captured: path defaults to "/", no attribute is invented.
    expect(cdpSetCookieParam({ name: "s", value: "v", domain: ".meta.ai" })).toEqual({
      name: "s",
      value: "v",
      domain: ".meta.ai",
      path: "/",
    });
  });

  test("persistent exporter session-expired + valid stored credential → jar restored from the stored credential", async () => {
    const { statePath, credentialPath } = await paths();
    const sets: MetaBrowserCookie[][] = [];
    let doc: MetaExporterDoc = okDoc;
    const deps = baseDeps({
      statePath,
      credentialPath,
      readLatest: async () => doc,
      readCredential: async () => storedCred(),
      validateCredential: async () => true,
      getBrowserCookies: async () => browserCookies(),
      setBrowserCookies: async (_cdpUrl, cookies) => {
        sets.push(cookies);
      },
    });

    const baseline = await renewMetaSession(deps);
    expect(baseline.status).toBe("valid");
    expect(baseline.restore.attempts).toBe(0);

    // First session-expired sighting: not yet persistent, no restore.
    doc = expiredDoc;
    const first = await renewMetaSession(deps);
    expect(first.status).toBe("valid");
    expect(first.restore.attempted).toBe(false);
    expect(sets).toHaveLength(0);

    // Second consecutive sighting: restore the jar from the stored
    // credential — the reverse of capture.
    const second = await renewMetaSession(deps);
    expect(second.status).toBe("valid");
    expect(second.alert).toBeNull();
    expect(second.restore.attempted).toBe(true);
    expect(second.restore.result).toBe("restored");
    expect(second.restore.attempts).toBe(1);
    expect(second.restore.lastAttemptAt).toBe(NOW.toISOString());
    expect(second.restore.nextAttemptAt).toBe(new Date(NOW.getTime() + 30 * 60 * 1000).toISOString());
    expect(sets).toHaveLength(1);
    expect(sets[0]?.map((c) => c.name)).toEqual(["session"]);
    expect(sets[0]?.[0]?.domain).toBe(".meta.ai");
    // The state file records the restore event without the value.
    const stateRaw = await readFile(statePath, "utf8");
    expect(stateRaw).toContain('"restore"');
    expect(stateRaw).not.toContain("stored-value");
  });

  test("restore backoff: no re-attempt inside the window, next attempt after it", async () => {
    const { statePath, credentialPath } = await paths();
    const sets: MetaBrowserCookie[][] = [];
    let nowMs = NOW.getTime();
    const deps = baseDeps({
      statePath,
      credentialPath,
      now: () => new Date(nowMs),
      readLatest: async () => expiredDoc,
      readCredential: async () => storedCred(),
      validateCredential: async () => true,
      setBrowserCookies: async (_cdpUrl, cookies) => {
        sets.push(cookies);
      },
    });

    await renewMetaSession(deps); // first sighting — no restore
    const first = await renewMetaSession(deps); // persistent — attempt 1
    expect(first.restore.attempts).toBe(1);
    expect(sets).toHaveLength(1);

    nowMs += 15 * 60 * 1000; // inside the 30 min backoff
    const backedOff = await renewMetaSession(deps);
    expect(backedOff.restore.attempted).toBe(false);
    expect(backedOff.restore.attempts).toBe(1);
    expect(sets).toHaveLength(1);

    nowMs += 16 * 60 * 1000; // 31 min after attempt 1 — backoff over
    const second = await renewMetaSession(deps);
    expect(second.restore.attempted).toBe(true);
    expect(second.restore.attempts).toBe(2);
    expect(second.restore.nextAttemptAt).toBe(new Date(nowMs + 2 * 3_600_000).toISOString());
    expect(sets).toHaveLength(2);
  });

  test("invalid stored credential → no restore attempted, attention stands (no restore loop)", async () => {
    const { statePath, credentialPath } = await paths();
    let setCalls = 0;
    const deps = baseDeps({
      statePath,
      credentialPath,
      readLatest: async () => expiredDoc,
      readCredential: async () => storedCred(),
      validateCredential: async () => false,
      getBrowserCookies: async () => browserCookies(),
      setBrowserCookies: async () => {
        setCalls += 1;
      },
    });
    const first = await renewMetaSession(deps);
    const second = await renewMetaSession(deps);
    expect(first.status).toBe("attention");
    expect(second.status).toBe("attention");
    expect(second.capture.result).toBe("validation-failed");
    expect(second.alert).toContain("Last-resort fallback");
    expect(setCalls).toBe(0);
    expect(second.restore.attempts).toBe(0);
  });

  test("restores that keep failing → attention after 3 attempts; exporter recovery resets the episode", async () => {
    const { statePath, credentialPath } = await paths();
    let setCalls = 0;
    let nowMs = NOW.getTime();
    let doc: MetaExporterDoc = expiredDoc;
    const deps = baseDeps({
      statePath,
      credentialPath,
      now: () => new Date(nowMs),
      readLatest: async () => doc,
      readCredential: async () => storedCred(),
      validateCredential: async () => true,
      setBrowserCookies: async () => {
        setCalls += 1;
        throw new Error("CDP Storage.setCookies failed");
      },
    });

    await renewMetaSession(deps); // first sighting — no restore
    const a1 = await renewMetaSession(deps);
    expect(a1.restore.result).toBe("restore-failed");
    expect(a1.status).toBe("valid");

    nowMs += 31 * 60 * 1000;
    const a2 = await renewMetaSession(deps);
    expect(a2.restore.attempts).toBe(2);
    expect(a2.status).toBe("valid");

    nowMs += 2 * 3_600_000 + 60 * 1000;
    const a3 = await renewMetaSession(deps);
    expect(a3.restore.attempts).toBe(3);
    expect(a3.status).toBe("attention");
    expect(a3.alert).toContain("restored the laptop Helium cookie jar");
    expect(setCalls).toBe(3);

    // The read path recovers: episode closes, alert clears. The
    // recovered exporter produces a fresh read at the current time —
    // an ok read measured hours ago would be stale readings under
    // the #15114 alert, not a recovery.
    doc = { status: "ok", measuredAt: new Date(nowMs).toISOString(), attemptedAt: new Date(nowMs).toISOString() };
    const healed = await renewMetaSession(deps);
    expect(healed.status).toBe("valid");
    expect(healed.alert).toBeNull();
    expect(healed.restore.attempts).toBe(0);
    expect(healed.restore.nextAttemptAt).toBeNull();
  });

  test("cadence capture finds a dead jar while the stored credential validates → restore attempted", async () => {
    const { statePath, credentialPath } = await paths();
    const sets: MetaBrowserCookie[][] = [];
    const state = await renewMetaSession(
      baseDeps({
        statePath,
        credentialPath,
        readCredential: async () => storedCred(SOON),
        validateCredential: async () => true,
        getBrowserCookies: async () => [{ name: "a", value: "b", domain: ".example.com" }],
        setBrowserCookies: async (_cdpUrl, cookies) => {
          sets.push(cookies);
        },
      }),
    );
    expect(state.status).toBe("valid");
    expect(state.capture.result).toBe("no-browser-session");
    expect(state.restore.attempted).toBe(true);
    expect(state.restore.result).toBe("restored");
    expect(sets).toHaveLength(1);
  });
});

describe("renewMetaSession — stale-readings alert (#15114)", () => {
  const expiredDoc: MetaExporterDoc = { status: "session-expired", measuredAt: null, attemptedAt: NOW.toISOString() };

  test("oracle: a valid credential does NOT mask stale readings — last good read older than STALE_MS raises attention + one push", async () => {
    const { statePath, credentialPath } = await paths();
    const pushes: string[] = [];
    let nowMs = NOW.getTime() - 2 * 3_600_000;
    let doc: MetaExporterDoc = {
      status: "ok",
      measuredAt: new Date(nowMs).toISOString(),
      attemptedAt: new Date(nowMs).toISOString(),
    };
    const deps = baseDeps({
      statePath,
      credentialPath,
      now: () => new Date(nowMs),
      readLatest: async () => doc,
      readCredential: async () => storedCred(),
      validateCredential: async () => true,
      notifyPush: async (notice) => {
        pushes.push(notice.title);
        return "sent:1/1";
      },
    });
    const baseline = await renewMetaSession(deps);
    expect(baseline.status).toBe("valid");
    expect(baseline.lastGoodAt).toBe(new Date(nowMs).toISOString());

    // Exporter goes session-expired; time passes beyond the stale bound.
    nowMs = NOW.getTime();
    doc = expiredDoc;
    const stale = await renewMetaSession(deps);
    expect(stale.status).toBe("attention");
    expect(stale.alert).toContain("readings are stale");
    expect(stale.alert).toContain("session-expired");
    expect(stale.notification.attempted).toBe(true);
    expect(pushes).toHaveLength(1);

    // Same episode, next cycle: still attention, no second push.
    const again = await renewMetaSession(deps);
    expect(again.status).toBe("attention");
    expect(again.notification.attempted).toBe(false);
    expect(pushes).toHaveLength(1);
  });

  test("oracle: the stale bound is exact — age == STALE_MS stays valid, STALE_MS + 1ms raises attention", async () => {
    const { statePath, credentialPath } = await paths();
    let nowMs = NOW.getTime() - STALE_MS;
    let doc: MetaExporterDoc = {
      status: "ok",
      measuredAt: new Date(nowMs).toISOString(),
      attemptedAt: new Date(nowMs).toISOString(),
    };
    const deps = baseDeps({
      statePath,
      credentialPath,
      now: () => new Date(nowMs),
      readLatest: async () => doc,
      readCredential: async () => storedCred(),
      validateCredential: async () => true,
    });
    await renewMetaSession(deps);
    doc = expiredDoc;
    nowMs = NOW.getTime();
    const atBound = await renewMetaSession(deps);
    expect(atBound.status).toBe("valid");
    expect(atBound.alert).toBeNull();

    nowMs = NOW.getTime() + 1;
    const pastBound = await renewMetaSession(deps);
    expect(pastBound.status).toBe("attention");
    expect(pastBound.alert).toContain("readings are stale");
  });

  test("oracle: a fresh ok exporter read is never stale, however old the previous last-good was", async () => {
    const { statePath, credentialPath } = await paths();
    let nowMs = NOW.getTime() - 24 * 3_600_000;
    const deps = baseDeps({
      statePath,
      credentialPath,
      now: () => new Date(nowMs),
      readLatest: async () => ({
        status: "ok",
        measuredAt: new Date(nowMs).toISOString(),
        attemptedAt: new Date(nowMs).toISOString(),
      }),
      readCredential: async () => storedCred(),
      validateCredential: async () => true,
    });
    await renewMetaSession(deps);
    nowMs = NOW.getTime();
    const fresh = await renewMetaSession({
      ...deps,
      readLatest: async () => ({ status: "ok", measuredAt: NOW.toISOString(), attemptedAt: NOW.toISOString() }),
    });
    expect(fresh.status).toBe("valid");
    expect(fresh.alert).toBeNull();
    expect(fresh.lastGoodAt).toBe(NOW.toISOString());
  });
});
