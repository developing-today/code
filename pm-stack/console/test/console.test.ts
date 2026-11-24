// pm-console: who gets in, what each operation runs, and what the HTTP
// surface refuses.
//
// Oracles: tokens are signed here with keys generated here, so "valid"
// is defined by the test and not by the code under test; every
// operation runs against a recording runner, so what would reach the
// machine is compared as an exact argument vector; and seeded generators
// drive the properties (any changed token byte is refused; no generated
// unit name outside the allowlist reaches systemctl; the unit parser
// reads back what a formatter wrote). A request is local only by the
// connection's own address, which the tests pass in as the server does.
// The dashboard behind the gate is a recorder too: what it was sent is
// compared with literal URLs, so no spelling of a path can pass as one
// of the public files unless the dashboard is asked for exactly that file.
import { beforeEach, describe, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { actions } from "../src/actions.ts";
import {
  AuthError,
  authorize,
  cameThroughEdge,
  clearJwksCache,
  identify,
  isLoopbackAddress,
  verifyJwt,
} from "../src/auth.ts";
import { type Config, ConfigError, type Issuer, readConfig } from "../src/config.ts";
import { HOLDABLE } from "../src/edge-proto.ts";
import { manageable, OpError, Ops, parseUnitFiles, parseUnits, type Run, triagePacket } from "../src/ops.ts";
import { assetLinks, type Deps, handle } from "../src/server.ts";

const NOW = 1_800_000_000_000;
const ACCESS: Issuer = {
  name: "cloudflare-access",
  issuer: "https://team.cloudflareaccess.com",
  jwksUri: "https://team.cloudflareaccess.com/cdn-cgi/access/certs",
  audience: "aud-tag",
  source: "access",
};
const CENTRAL: Issuer = {
  name: "central",
  issuer: "https://id.example",
  jwksUri: "https://id.example/jwks",
  audience: "pm-console",
  source: "bearer",
};
const OWNER = "owner@example.com";

function b64u(bytes: Uint8Array | string): string {
  const raw = typeof bytes === "string" ? new TextEncoder().encode(bytes) : bytes;
  return btoa(String.fromCharCode(...raw))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

interface Signer {
  jwk: JsonWebKey & { kid: string };
  sign(claims: Record<string, unknown>, header?: Record<string, unknown>): Promise<string>;
}

async function signer(alg: "RS256" | "ES256", kid: string): Promise<Signer> {
  const params =
    alg === "RS256"
      ? { name: "RSASSA-PKCS1-v1_5", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" }
      : { name: "ECDSA", namedCurve: "P-256" };
  const pair = (await crypto.subtle.generateKey(params, true, ["sign", "verify"])) as CryptoKeyPair;
  const jwk = { ...(await crypto.subtle.exportKey("jwk", pair.publicKey)), kid };
  return {
    jwk,
    async sign(claims, header = {}) {
      const head = b64u(JSON.stringify({ alg, kid, typ: "JWT", ...header }));
      const body = b64u(JSON.stringify(claims));
      const sig = await crypto.subtle.sign(
        alg === "RS256" ? "RSASSA-PKCS1-v1_5" : { name: "ECDSA", hash: "SHA-256" },
        pair.privateKey,
        new TextEncoder().encode(`${head}.${body}`),
      );
      return `${head}.${body}.${b64u(new Uint8Array(sig))}`;
    },
  };
}

function keyServer(sets: Record<string, JsonWebKey[]>, gate?: Promise<void>) {
  const calls: string[] = [];
  const signals: (AbortSignal | undefined)[] = [];
  const fetchImpl = async (url: string, init?: { signal?: AbortSignal }) => {
    calls.push(url);
    signals.push(init?.signal);
    await gate;
    return { ok: url in sets, status: url in sets ? 200 : 404, json: async () => ({ keys: sets[url] ?? [] }) };
  };
  return { fetchImpl, calls, signals };
}

const good = (issuer: Issuer, extra: Record<string, unknown> = {}) => ({
  iss: issuer.issuer,
  aud: [issuer.audience],
  exp: NOW / 1000 + 600,
  iat: NOW / 1000 - 5,
  sub: "user-1",
  email: OWNER,
  ...extra,
});

function rng(seed: number) {
  let s = seed >>> 0;
  return () => (s = (Math.imul(s, 1664525) + 1013904223) >>> 0) / 2 ** 32;
}

let rsa: Signer;
let ec: Signer;
let other: Signer;
beforeEach(async () => {
  clearJwksCache();
  rsa ??= await signer("RS256", "k1");
  ec ??= await signer("ES256", "k2");
  other ??= await signer("RS256", "k1"); // same key id, different key
});

describe("token verification", () => {
  test("accepts a token its issuer signed, RS256 and ES256", async () => {
    const keys = keyServer({ [ACCESS.jwksUri]: [rsa.jwk], [CENTRAL.jwksUri]: [ec.jwk] });
    expect((await verifyJwt(await rsa.sign(good(ACCESS)), ACCESS, keys.fetchImpl, NOW)).email).toBe(OWNER);
    expect(
      (await verifyJwt(await ec.sign(good(CENTRAL, { aud: CENTRAL.audience })), CENTRAL, keys.fetchImpl, NOW)).sub,
    ).toBe("user-1");
  });

  test("refuses each way a token can be wrong", async () => {
    const keys = keyServer({ [ACCESS.jwksUri]: [rsa.jwk] });
    const refuse = async (token: string, why: RegExp) => {
      await expect(verifyJwt(token, ACCESS, keys.fetchImpl, NOW)).rejects.toThrow(why);
    };
    await refuse(await rsa.sign(good(ACCESS, { iss: "https://evil.cloudflareaccess.com" })), /issuer/);
    await refuse(await rsa.sign(good(ACCESS, { aud: ["another-app"] })), /audience/);
    await refuse(await rsa.sign(good(ACCESS, { exp: NOW / 1000 - 120 })), /expired/);
    await refuse(await rsa.sign({ ...good(ACCESS), exp: undefined }), /expired/);
    await refuse(await rsa.sign(good(ACCESS, { nbf: NOW / 1000 + 600 })), /not yet valid/);
    await refuse(await rsa.sign(good(ACCESS, { iat: NOW / 1000 + 600 })), /future/);
    await refuse(await other.sign(good(ACCESS)), /signature/);
    await refuse(await rsa.sign(good(ACCESS), { kid: "unknown" }), /no key/);
    await refuse(await rsa.sign(good(ACCESS), { alg: "none" }), /algorithm/);
    await refuse(await rsa.sign(good(ACCESS), { alg: "HS256" }), /algorithm/);
    await refuse("a.b", /malformed/);
    await refuse("!!.??.**", /malformed/);
    // A token asking for ES256 against an RSA key of the same id.
    await refuse(await rsa.sign(good(ACCESS), { alg: "ES256" }), /no key/);
  });

  test("property: changing any one character of a valid token refuses it", async () => {
    const keys = keyServer({ [ACCESS.jwksUri]: [rsa.jwk] });
    const token = await rsa.sign(good(ACCESS));
    const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    const next = rng(12268);
    for (let i = 0; i < 300; i++) {
      const at = Math.floor(next() * token.length);
      if (token[at] === ".") continue;
      let c = alphabet[Math.floor(next() * alphabet.length)];
      if (c === token[at]) c = c === "A" ? "B" : "A";
      const changed = token.slice(0, at) + c + token.slice(at + 1);
      // base64url's last character of a segment can carry unused bits:
      // a change there may decode to the same bytes, which is the same token.
      const same = changed.split(".").every((part, n) => atob(pad(part)) === atob(pad(token.split(".")[n])));
      if (same) continue;
      await expect(verifyJwt(changed, ACCESS, keys.fetchImpl, NOW)).rejects.toThrow();
    }
    function pad(s: string) {
      return s.replace(/-/g, "+").replace(/_/g, "/") + "=".repeat((4 - (s.length % 4)) % 4);
    }
  });

  test("caches the key set, and fetches a rotated key at most once per cooldown", async () => {
    const rotated = await signer("RS256", "k9");
    const sets: Record<string, JsonWebKey[]> = { [ACCESS.jwksUri]: [rsa.jwk] };
    const keys = keyServer(sets);
    await verifyJwt(await rsa.sign(good(ACCESS)), ACCESS, keys.fetchImpl, NOW);
    await verifyJwt(await rsa.sign(good(ACCESS)), ACCESS, keys.fetchImpl, NOW + 1000);
    expect(keys.calls.length).toBe(1);
    expect(keys.signals[0]).toBeInstanceOf(AbortSignal); // the fetch has a deadline
    sets[ACCESS.jwksUri] = [rsa.jwk, rotated.jwk];
    // Tokens naming an unknown key cost nothing while the last fetch is
    // fresh: anyone can send them, so they must not each reach the issuer.
    const unknown = await rotated.sign(good(ACCESS));
    for (let i = 0; i < 20; i++) {
      await expect(verifyJwt(unknown, ACCESS, keys.fetchImpl, NOW + 2000 + i)).rejects.toThrow(/no key matches/);
    }
    expect(keys.calls.length).toBe(1);
    // Once the cooldown has passed, the rotated key is picked up.
    await verifyJwt(unknown, ACCESS, keys.fetchImpl, NOW + 31_000);
    expect(keys.calls.length).toBe(2);
    await verifyJwt(unknown, ACCESS, keys.fetchImpl, NOW + 32_000);
    expect(keys.calls.length).toBe(2);
  });

  test("requests arriving together share one key fetch", async () => {
    let open = () => {};
    const gate = new Promise<void>((resolve) => {
      open = resolve;
    });
    const keys = keyServer({ [ACCESS.jwksUri]: [rsa.jwk] }, gate);
    const token = await rsa.sign(good(ACCESS));
    const all = Promise.all(Array.from({ length: 25 }, () => verifyJwt(token, ACCESS, keys.fetchImpl, NOW)));
    await new Promise((resolve) => setTimeout(resolve, 20)); // let every verification reach the key lookup
    open();
    expect((await all).length).toBe(25);
    expect(keys.calls.length).toBe(1);
    // A failed fetch is not remembered: the next request tries again.
    clearJwksCache();
    const down = keyServer({});
    await expect(verifyJwt(token, ACCESS, down.fetchImpl, NOW)).rejects.toThrow(/key set/);
    await verifyJwt(token, ACCESS, keys.fetchImpl, NOW);
  });
});

describe("who is the owner", () => {
  const cfg = { owners: [OWNER], certNames: ["laptop.devices.central"], issuers: [ACCESS, CENTRAL] };
  const edge = (headers: Record<string, string> = {}) =>
    new Request("https://pm.security.cab/console/api/me", {
      headers: { host: "pm.security.cab", "cf-ray": "1", ...headers },
    });
  // The tunnel connects from this machine, so an edge request's peer is loopback too.
  const at = (keys: { fetchImpl: ReturnType<typeof keyServer>["fetchImpl"] }) => ({
    fetchImpl: keys.fetchImpl,
    now: NOW,
    peer: "127.0.0.1",
  });

  test("the owner's verified address gets in through either issuer", async () => {
    const keys = keyServer({ [ACCESS.jwksUri]: [rsa.jwk], [CENTRAL.jwksUri]: [ec.jwk] });
    expect(await identify(edge({ "cf-access-jwt-assertion": await rsa.sign(good(ACCESS)) }), cfg, at(keys))).toEqual({
      via: "cloudflare-access",
      email: OWNER,
      sub: "user-1",
    });
    expect(
      await identify(
        edge({ cookie: `x=1; CF_Authorization=${await rsa.sign(good(ACCESS, { email: "Owner@Example.COM" }))}` }),
        cfg,
        at(keys),
      ),
    ).toMatchObject({ email: OWNER });
    expect(
      await identify(edge({ authorization: `Bearer ${await ec.sign(good(CENTRAL))}` }), cfg, at(keys)),
    ).toMatchObject({ via: "central", email: OWNER });
  });

  test("anyone else is refused, by status", async () => {
    const keys = keyServer({ [ACCESS.jwksUri]: [rsa.jwk], [CENTRAL.jwksUri]: [ec.jwk] });
    const status = async (req: Request) =>
      identify(req, cfg, at(keys)).then(
        () => 200,
        (e: AuthError) => e.status,
      );
    expect(await status(edge())).toBe(401);
    expect(await status(edge({ "cf-access-jwt-assertion": "garbage" }))).toBe(401);
    expect(await status(edge({ "cf-access-jwt-assertion": await other.sign(good(ACCESS)) }))).toBe(401);
    expect(
      await status(edge({ "cf-access-jwt-assertion": await rsa.sign(good(ACCESS, { email: "someone@example.net" })) })),
    ).toBe(403);
    expect(
      await status(edge({ "cf-access-jwt-assertion": await rsa.sign(good(ACCESS, { email_verified: false })) })),
    ).toBe(403);
    // A token for one issuer is not accepted in another issuer's place.
    expect(await status(edge({ authorization: `Bearer ${await rsa.sign(good(ACCESS))}` }))).toBe(401);
    // With no owners configured, a valid token still opens nothing.
    const closed = { ...cfg, owners: [], certNames: [] };
    await expect(
      identify(edge({ "cf-access-jwt-assertion": await rsa.sign(good(ACCESS)) }), closed, at(keys)),
    ).rejects.toThrow(AuthError);
  });

  test("a client certificate the edge checked is the owner by its name", () => {
    expect(authorize({ common_name: "laptop.devices.central", sub: "" }, "cloudflare-access", cfg)).toEqual({
      via: "cloudflare-access",
      certName: "laptop.devices.central",
      sub: "",
    });
    expect(() => authorize({ common_name: "other.devices.central" }, "cloudflare-access", cfg)).toThrow(AuthError);
  });

  test("only a connection from this machine that crossed no proxy is local", async () => {
    const local = (headers: Record<string, string>) => new Request("http://127.0.0.1:4872/console/api/me", { headers });
    const here = { peer: "127.0.0.1" };
    for (const peer of ["127.0.0.1", "127.8.9.10", "::1", "::ffff:127.0.0.1"]) {
      expect(await identify(local({ host: "127.0.0.1:4872" }), cfg, { peer })).toEqual({
        via: "loopback",
        email: "local",
      });
    }
    expect(cameThroughEdge(local({ host: "localhost:4872" }))).toBe(false);
    // The Host header is the caller's to write. A connection from another
    // machine that names loopback in it is not local, whatever it says.
    for (const peer of ["100.99.113.103", "203.0.113.9", "::ffff:203.0.113.9", "fe80::1", "", null, undefined]) {
      await expect(identify(local({ host: "127.0.0.1:4872" }), cfg, { peer })).rejects.toThrow(/sign-in required/);
    }
    await expect(identify(local({ host: "127.0.0.1:4872" }), cfg)).rejects.toThrow(/sign-in required/);
    for (const address of [
      "127.0.0.1.evil.example",
      "1127.0.0.1",
      "127.0.0.1:80",
      "localhost",
      "0.0.0.0",
      "::",
      "::ffff:",
    ]) {
      expect(isLoopbackAddress(address)).toBe(false);
    }
    for (const headers of [
      { host: "pm.security.cab" },
      { host: "evil.example:4872" }, // a rebound name resolving to loopback
      { host: "127.0.0.1:4872", "cf-connecting-ip": "203.0.113.9" },
      { host: "127.0.0.1:4872", "x-forwarded-for": "203.0.113.9" },
      { host: "127.0.0.1:4872", "cdn-loop": "cloudflare" },
    ] as Record<string, string>[]) {
      await expect(identify(local(headers), cfg, here)).rejects.toThrow(/sign-in required/);
    }
  });
});

describe("configuration", () => {
  test("defaults, the Access issuer and the issuers file", () => {
    const file = JSON.stringify([
      { name: "central", issuer: "https://id.example", jwksUri: "https://id.example/jwks", audience: "pm-console" },
    ]);
    const cfg = readConfig(
      {
        HOME: "/h",
        PM_CONSOLE_OWNER_EMAILS: " Owner@Example.com ,",
        PM_CONSOLE_ACCESS_TEAM_DOMAIN: "team.cloudflareaccess.com",
        PM_CONSOLE_ACCESS_AUD: "aud-tag",
        PM_CONSOLE_ISSUERS_FILE: "/x",
      },
      () => file,
    );
    expect(cfg.port).toBe(4872);
    expect(cfg.bind).toBe("127.0.0.1");
    expect(cfg.owners).toEqual([OWNER]);
    expect(cfg.issuers).toEqual([ACCESS, CENTRAL]);
    expect(cfg.auditFile).toBe("/h/.local/state/pm-console/audit.jsonl");
  });

  test("a bad value stops the boot and names the variable", () => {
    for (const [env, why] of [
      [{ PM_CONSOLE_PORT: "80abc" }, /PM_CONSOLE_PORT/],
      [{ PM_CONSOLE_PORT: "0" }, /PM_CONSOLE_PORT/],
      [{ PM_CONSOLE_PORT: "70000" }, /PM_CONSOLE_PORT/],
      [{ PM_CONSOLE_ACCESS_AUD: "x" }, /set together/],
      [{ PM_CONSOLE_ACCESS_TEAM_DOMAIN: "evil.example", PM_CONSOLE_ACCESS_AUD: "x" }, /team domain/],
      [{ PM_CONSOLE_ANDROID_SHA256: "AB:CD" }, /fingerprint/],
      // A request with no token is trusted when it comes from this
      // machine, so the listener must not be reachable from another.
      [{ PM_CONSOLE_BIND: "0.0.0.0" }, /PM_CONSOLE_BIND/],
      [{ PM_CONSOLE_BIND: "100.99.113.103" }, /PM_CONSOLE_BIND/],
      [{ PM_CONSOLE_BIND: "::" }, /PM_CONSOLE_BIND/],
    ] as const) {
      expect(() => readConfig(env)).toThrow(why);
    }
    expect(() =>
      readConfig(
        { PM_CONSOLE_ISSUERS_FILE: "/x" },
        () => '[{"name":"c","issuer":"http://id.example","jwksUri":"https://id.example/jwks","audience":"a"}]',
      ),
    ).toThrow(/https/);
    expect(() => readConfig({ PM_CONSOLE_ISSUERS_FILE: "/x" }, () => "{}")).toThrow(ConfigError);
    // What the machine is called where machines are listed: what the
    // environment says, made fit to show; unsaid, the host's own name is
    // used when the console starts.
    expect(readConfig({ PM_CONSOLE_MACHINE: "nixos" }).machineName).toBe("nixos");
    expect(readConfig({ PM_CONSOLE_MACHINE: " Drew's laptop " }).machineName).toBe("Drew-s laptop");
    expect(readConfig({}).machineName).toBe("");
    for (const bind of ["127.0.0.1", "::1", "localhost"]) {
      expect(readConfig({ PM_CONSOLE_BIND: bind }).bind).toBe(bind);
    }
    // The dashboard has no sign-in of its own: only an address on this machine can stand behind the gate.
    expect(readConfig({}).dashboardUrl).toBe("http://127.0.0.1:4870");
    expect(readConfig({ PM_CONSOLE_DASHBOARD_URL: "http://localhost:9/" }).dashboardUrl).toBe("http://localhost:9");
    expect(readConfig({ PM_CONSOLE_DASHBOARD_URL: "" }).dashboardUrl).toBeNull();
    for (const url of [
      "http://100.99.113.103:4870",
      "https://127.0.0.1:4870",
      "http://127.0.0.1:4870/admin",
      "http://127.0.0.1:4870/?x=1",
      "http://user:pw@127.0.0.1:4870",
      "http://127.0.0.1.evil.example:4870",
      "http://evil.example",
      "4870",
    ]) {
      expect(() => readConfig({ PM_CONSOLE_DASHBOARD_URL: url })).toThrow(/PM_CONSOLE_DASHBOARD_URL/);
    }
  });
});

function recorder(answers: (argv: string[]) => string | { code: number; stderr: string } = () => "") {
  const calls: { argv: string[]; stdin?: string }[] = [];
  const run: Run = async (argv, opts) => {
    calls.push({ argv, stdin: opts?.stdin });
    const a = answers(argv);
    return typeof a === "string" ? { code: 0, stdout: a, stderr: "" } : { code: a.code, stdout: "", stderr: a.stderr };
  };
  return { run, calls };
}

const CFG: Config = readConfig({
  HOME: "/h",
  PM_CONSOLE_OWNER_EMAILS: OWNER,
  PM_CONSOLE_ANDROID_SHA256: Array(32).fill("AB").join(":"),
});

describe("operations", () => {
  test("deploys and builds are read from GitHub's own record, and started again from main", async () => {
    // Oracle: the two answers are GitHub's own shape for a workflow's
    // runs, and what is expected of them is written out by hand.
    const siteRuns = {
      workflow_runs: [
        {
          id: 45120541236,
          head_sha: "ca5c04493289cde76fa0d7f287bbd46e0395c491",
          status: "completed",
          conclusion: "success",
          event: "push",
          created_at: "2026-11-23T05:51:40Z",
          html_url: "https://github.com/developing-today/code/actions/runs/45120541236",
        },
        {
          id: 45119999999,
          head_sha: "326ac803beddb586530c6e48b76ce1db630e8aca",
          status: "in_progress",
          conclusion: null,
          event: "workflow_dispatch",
          created_at: "2026-11-23T05:42:59Z",
          html_url: "https://github.com/developing-today/code/actions/runs/45119999999",
        },
      ],
    };
    const r = recorder((argv) => {
      const path = argv.find((a) => a.startsWith("repos/")) ?? "";
      if (path.includes("pm-console-edge-deploy.yml/runs")) return JSON.stringify(siteRuns);
      if (path.includes("pm-console-android.yml/runs")) return { code: 1, stderr: "HTTP 502: Bad Gateway" };
      return "";
    });
    const ops = new Ops(CFG, r.run);
    const got = await ops.releases();
    expect(got.site).toEqual([
      {
        id: 45120541236,
        commit: "ca5c04493289",
        state: "success",
        byHand: false,
        at: "2026-11-23T05:51:40Z",
        url: "https://github.com/developing-today/code/actions/runs/45120541236",
      },
      {
        id: 45119999999,
        commit: "326ac803bedd",
        state: "in_progress", // not over: where it is, not how it ended
        byHand: true,
        at: "2026-11-23T05:42:59Z",
        url: "https://github.com/developing-today/code/actions/runs/45119999999",
      },
    ]);
    // The app's runs could not be read: that is said for them alone, and the site's still show.
    expect((got.app as { error: string }).error).toContain("502");
    // Only finished-or-not runs on main are asked for, a handful of each.
    expect(r.calls.map((c) => c.argv.slice(1))).toEqual([
      [
        "gh",
        "api",
        "repos/developing-today/code/actions/workflows/pm-console-edge-deploy.yml/runs?branch=main&per_page=5",
      ],
      ["gh", "api", "repos/developing-today/code/actions/workflows/pm-console-android.yml/runs?branch=main&per_page=5"],
    ]);

    // Starting one again is the workflow's own start, on main, and nothing else.
    const w = recorder();
    const table = actions(new Ops(CFG, w.run), {} as never);
    expect(await table["deploy.start"].run({ what: "site" })).toEqual({ started: "site", scratch: false });
    expect(await table["deploy.start"].run({ what: "site", scratch: true })).toEqual({
      started: "site",
      scratch: true,
    });
    expect(await table["deploy.start"].run({ what: "app", scratch: true })).toEqual({ started: "app", scratch: false });
    const at = (workflow: string) => [
      "gh",
      "api",
      "-X",
      "POST",
      `repos/developing-today/code/actions/workflows/${workflow}/dispatches`,
      "--input",
      "-",
    ];
    expect(w.calls.map((c) => [c.argv.slice(1), JSON.parse(c.stdin ?? "null")])).toEqual([
      [at("pm-console-edge-deploy.yml"), { ref: "main" }],
      [at("pm-console-edge-deploy.yml"), { ref: "main", inputs: { scratch: "true" } }],
      [at("pm-console-android.yml"), { ref: "main" }], // the app's build has no such input
    ]);
    // Anything else is refused before GitHub is asked: no other workflow, branch or input can be named.
    for (const what of ["pm-dispatch.yml", "../x", "", null, { site: true }, "SITE"]) {
      await expect(table["deploy.start"].run({ what })).rejects.toThrow(/site or app/);
    }
    expect(w.calls.length).toBe(3);
    // What is recorded of it is which of the two, and whether from nothing; and it is never held to be done late.
    expect(table["deploy.start"].audit({ what: "site", scratch: true, extra: "x" })).toEqual({
      what: "site",
      scratch: true,
    });
    expect(HOLDABLE.has("deploy.start")).toBe(false);
  });

  test("queue actions run the fleet verb with the reason as one argument", async () => {
    const r = recorder();
    const ops = new Ops(CFG, r.run);
    await ops.queueAct("unblock", "20306", "--force; rm -rf ~  it can proceed");
    expect(r.calls).toEqual([
      {
        argv: [
          "fleet",
          "queue",
          "unblock",
          "20306",
          "--worker",
          "pm-console",
          "--reason",
          "--force; rm -rf ~  it can proceed",
        ],
        stdin: undefined,
      },
    ]);
    for (const [action, task, reason] of [
      ["delete", "1", "x"],
      ["unblock", "1; id", "x"],
      ["unblock", "-1", "x"],
      ["unblock", "0", "x"],
      ["cancel", "5", "  "],
      ["cancel", "5", 7],
    ]) {
      await expect(ops.queueAct(action, task, reason)).rejects.toThrow(OpError);
    }
    expect(r.calls.length).toBe(1);
  });

  test("a failed deferred prompt enqueue is reported, not recorded done", async () => {
    // The laptop runs this late, with nobody listening for the answer:
    // a nonzero enqueue must throw, so the held action is reported
    // failed and the edge does not acknowledge a queue task that was
    // never filed.
    const r = recorder(() => ({ code: 1, stderr: "queue is down" }));
    const ops = new Ops(CFG, r.run);
    const table = actions(ops, {} as never);
    await expect(
      table["prompt.enqueue"].run({ issue: 201, text: "Do the docs", at: "2026-11-22T12:00:00Z" }),
    ).rejects.toThrow(/queue is down/);
    // And the good path still answers queued.
    const ok = recorder(() => ({ code: 0, stderr: "" }));
    const good = actions(new Ops(CFG, ok.run), {} as never);
    await expect(
      good["prompt.enqueue"].run({ issue: 201, text: "Do the docs", at: "2026-11-22T12:00:00Z" }),
    ).resolves.toEqual({ issue: 201, queued: true });
  });

  test("the queue is read with bound parameters and never written", async () => {
    const r = recorder(() => "secret-token\n");
    const sent: { url: string; body: { requests: { stmt?: { sql: string; args: unknown[] } }[] }; auth: string }[] = [];
    const ops = new Ops(CFG, r.run, async (url, init) => {
      sent.push({ url, body: JSON.parse(init.body as string), auth: init.headers.Authorization });
      return {
        ok: true,
        status: 200,
        json: async () => ({
          results: [
            {
              type: "ok",
              response: {
                result: { cols: [{ name: "id" }, { name: "title" }], rows: [[{ value: "7" }, { value: "t" }]] },
              },
            },
          ],
        }),
      };
    });
    expect(await ops.queueList("blocked", "50%_off'; DROP TABLE tasks;--", "10")).toEqual([{ id: "7", title: "t" }]);
    const stmt = sent[0].body.requests[0].stmt!;
    expect(stmt.sql.startsWith("SELECT")).toBe(true);
    expect(stmt.sql).not.toContain("DROP");
    expect(stmt.args).toEqual([
      { type: "text", value: "repo-health-code-pilot" },
      { type: "text", value: "blocked" },
      { type: "text", value: "%50\\%\\_off'; DROP TABLE tasks;--%" },
      { type: "integer", value: "10" },
    ]);
    expect(sent[0].auth).toBe("Bearer secret-token");
    await ops.queueCounts();
    expect(r.calls.length).toBe(1); // the token is fetched once
    await expect(ops.queueList("done", "", "")).rejects.toThrow(/status/);
  });

  // Oracle: the store's own wire format (Hrana over HTTP, as Turso
  // documents it). A whole number is sent as text marked as an integer,
  // because the wire carries 64 bits and a JSON number does not. Seen
  // live on 2026-11-24: an issue's number arrived as "7920", and a
  // screen that wanted a number showed no issue at all.
  test("a whole number comes from the store as marked text and is read as the number it is", async () => {
    const r = recorder(() => "secret-token\n");
    const ops = new Ops(CFG, r.run, async () => ({
      ok: true,
      status: 200,
      json: async () => ({
        results: [
          {
            type: "ok",
            response: {
              result: {
                cols: ["id", "big", "below", "title", "ratio", "nothing", "odd", "empty", "bare"].map((name) => ({
                  name,
                })),
                rows: [
                  [
                    { type: "integer", value: "7920" },
                    // One more than a number can hold exactly: it stays the text it came as.
                    { type: "integer", value: "9007199254740993" },
                    { type: "integer", value: "-3" },
                    // Text that looks like a number is text.
                    { type: "text", value: "123" },
                    { type: "float", value: 0.5 },
                    { type: "null" },
                    // Marked as an integer and not one: passed on as it came, not made into nonsense.
                    { type: "integer", value: "seven" },
                    { type: "integer", value: "" },
                    // Not marked at all (a stand-in, or another store): as it came.
                    { value: "7" },
                  ],
                ],
              },
            },
          },
        ],
      }),
    }));
    expect(await ops.sql("SELECT 1", [])).toEqual([
      {
        id: 7920,
        big: "9007199254740993",
        below: -3,
        title: "123",
        ratio: 0.5,
        nothing: null,
        odd: "seven",
        empty: "",
        bare: "7",
      },
    ]);
  });

  test("a rotated queue token is fetched again, once", async () => {
    let issued = 0;
    const r = recorder(() => `token-${++issued}\n`);
    const seen: string[] = [];
    const answer = (status: number) => ({
      ok: status === 200,
      status,
      json: async () => ({
        results: [{ type: "ok", response: { result: { cols: [{ name: "status" }, { name: "n" }], rows: [] } } }],
      }),
    });
    let valid = "token-2";
    const ops = new Ops(CFG, r.run, async (_url, init) => {
      seen.push(init.headers.Authorization);
      return answer(init.headers.Authorization === `Bearer ${valid}` ? 200 : 401);
    });
    await ops.queueCounts(); // token-1 is refused, token-2 works
    expect(seen).toEqual(["Bearer token-1", "Bearer token-2"]);
    await ops.queueCounts();
    expect(seen.length).toBe(3);
    expect(issued).toBe(2);
    // A store that keeps refusing gets one retry, not a loop.
    valid = "none";
    await expect(ops.queueCounts()).rejects.toThrow(/http 401/);
    expect(seen.length).toBe(5);
    expect(issued).toBe(3);
  });

  test("a message's text travels on stdin and targets are checked", async () => {
    const r = recorder();
    const ops = new Ops(CFG, r.run);
    await ops.send(["group:opencode", "agent:conductor"], "--command do the thing\n$(reboot)", true);
    expect(r.calls[0]).toEqual({
      argv: ["fleet", "bridge", "send", "--to", "group:opencode,agent:conductor", "--command", "-"],
      stdin: "--command do the thing\n$(reboot)",
    });
    await ops.send(["all"], "an idea", false);
    expect(r.calls[1].argv).toEqual(["fleet", "bridge", "send", "--to", "all", "-"]);
    for (const to of [[], ["--to"], ["a,b"], ["agent:"], [""], "all", ["x y"]]) {
      await expect(ops.send(to, "text", false)).rejects.toThrow(OpError);
    }
    await expect(ops.send(["all"], "", false)).rejects.toThrow(/required/);
    expect(r.calls.length).toBe(2);
  });

  test("ranking an issue adds the new rank before it removes the others", async () => {
    const r = recorder((argv) =>
      argv.includes("repos/developing-today/code/issues/12279")
        ? JSON.stringify({ labels: [{ name: "now" }, { name: "app/id" }, { name: "priority/high" }] })
        : "{}",
    );
    const ops = new Ops(CFG, r.run);
    await ops.setRank("12279", "next");
    expect(r.calls.map((c) => c.argv.slice(3))).toEqual([
      ["repos/developing-today/code/issues/12279"],
      ["-X", "POST", "repos/developing-today/code/issues/12279/labels", "--input", "-"],
      ["-X", "DELETE", "repos/developing-today/code/issues/12279/labels/now"],
      ["-X", "DELETE", "repos/developing-today/code/issues/12279/labels/priority%2Fhigh"],
    ]);
    expect(r.calls[1].stdin).toBe('{"labels":["next"]}');
    // If removing fails part way, the issue still carries the new rank.
    const failing = recorder((argv) =>
      argv.includes("DELETE")
        ? { code: 1, stderr: "gh: Server Error (HTTP 502)" }
        : JSON.stringify({ labels: [{ name: "now" }] }),
    );
    await expect(new Ops(CFG, failing.run).setRank("12279", "next")).rejects.toThrow(/502/);
    expect(failing.calls.map((c): string | undefined => c.argv[4])).toEqual([undefined, "POST", "DELETE"]);
    // A rank the issue already has is not added again; "none" only removes.
    const same = recorder(() => JSON.stringify({ labels: [{ name: "now" }, { name: "next" }] }));
    await new Ops(CFG, same.run).setRank("12279", "now");
    expect(same.calls.map((c): string | undefined => c.argv[4])).toEqual([undefined, "DELETE"]);
    const none = recorder(() => JSON.stringify({ labels: [{ name: "now" }] }));
    await new Ops(CFG, none.run).setRank("12279", "none");
    expect(none.calls.map((c): string | undefined => c.argv[4])).toEqual([undefined, "DELETE"]);
    await expect(ops.setRank("12279", "urgent")).rejects.toThrow(/rank/);
    await expect(ops.setRank("12279/labels", "now")).rejects.toThrow(OpError);
  });

  test("ranked issues come back most urgent first", async () => {
    const item = (number: number, ...labels: string[]) => ({
      number,
      title: `t${number}`,
      html_url: `u${number}`,
      updated_at: "2026-11-22T01:02:03Z",
      comments: 0,
      labels: labels.map((name) => ({ name })),
    });
    const r = recorder(() =>
      JSON.stringify({
        items: [
          item(5, "next"),
          item(9, "now", "priority/high"),
          item(2, "priority/high"),
          item(7, "expedite"),
          item(1, "now"),
        ],
      }),
    );
    const listed = await new Ops(CFG, r.run).priorities();
    expect(listed.truncated).toBe(false);
    expect(listed.issues.map((i) => [i.number, i.rank])).toEqual([
      [7, "expedite"],
      [1, "now"],
      [9, "now"],
      [2, "priority/high"],
      [5, "next"],
    ]);
    expect(r.calls.length).toBe(1);

    // Search order is not rank order: the one expedite issue can be on any
    // page, so every page is read before sorting.
    const pageOf = (argv: string[]) => Number(argv.find((a) => a.startsWith("page="))?.slice(5));
    const paged = (pages: number) =>
      recorder((argv) => {
        const page = pageOf(argv);
        const full = page < pages;
        return JSON.stringify({
          items: Array.from({ length: full ? 100 : 3 }, (_, n) =>
            item(page * 1000 + n, !full && n === 2 ? "expedite" : "next"),
          ),
        });
      });
    const three = paged(3);
    const all = await new Ops(CFG, three.run).priorities();
    expect(three.calls.map((c) => pageOf(c.argv))).toEqual([1, 2, 3]);
    expect(all.issues.length).toBe(203);
    expect(all.issues[0]).toMatchObject({ number: 3002, rank: "expedite" });
    expect(all.truncated).toBe(false);
    // Past the cap the list says it is incomplete instead of looking whole.
    const endless = paged(Number.POSITIVE_INFINITY);
    const capped = await new Ops(CFG, endless.run).priorities();
    expect(endless.calls.length).toBe(10);
    expect(capped.issues.length).toBe(1000);
    expect(capped.truncated).toBe(true);
    // Exactly one full page: the second, empty page ends the read.
    const exact = recorder((argv) =>
      JSON.stringify({ items: pageOf(argv) === 1 ? Array.from({ length: 100 }, (_, n) => item(n + 1, "next")) : [] }),
    );
    expect(await new Ops(CFG, exact.run).priorities()).toMatchObject({ truncated: false });
    expect(exact.calls.length).toBe(2);
  });

  test("an idea is filed and queued in the same pass", async () => {
    const packets: unknown[] = [];
    const answers = (enqueue: string | { code: number; stderr: string }) => (argv: string[]) => {
      if (argv[1] === "gh")
        return JSON.stringify({ number: 12400, html_url: "https://x/12400", title: "Join at admission" });
      packets.push(JSON.parse(readFileSync(argv[3], "utf8")));
      return enqueue;
    };
    const r = recorder(answers("{}"));
    const ops = new Ops(CFG, r.run);
    expect(await ops.fileIdea("  Join at admission ", "details", "now")).toEqual({
      issue: 12400,
      url: "https://x/12400",
      queued: true,
    });
    expect(r.calls[0].argv.slice(3)).toEqual(["-X", "POST", "repos/developing-today/code/issues", "--input", "-"]);
    expect(JSON.parse(r.calls[0].stdin!)).toEqual({ title: "Join at admission", body: "details", labels: ["now"] });
    expect(r.calls[1].argv.slice(0, 3)).toEqual(["fleet", "queue", "enqueue"]);
    expect(r.calls[1].argv.slice(4)).toEqual(["--queue", "repo-health-code-pilot"]);
    expect(() => readFileSync(r.calls[1].argv[3])).toThrow(); // the packet file does not outlive the call
    expect(packets[0]).toMatchObject({
      title: "Triage developing-today/code#12400: Join at admission",
      priority: 20,
      dedupe_key: "repo-health:v1:triage:developing-today/code:issue:12400",
      payload: {
        schema: "repo-task/v1",
        kind: "triage",
        repo: "developing-today/code",
        queue: "repo-health-code-pilot",
        issue: { number: 12400, url: "https://github.com/developing-today/code/issues/12400" },
        max_attempts: 1000,
      },
    });
    // The queue only takes priorities on its scale, and a more urgent
    // rank never gets a later place than a less urgent one.
    const scale = [10, 20, 30, 40, 50, 70, 90];
    const order = (["expedite", "priority/critical", "now", "priority/high", "next", "none"] as const).map(
      (rank) => triagePacket("o/r", "q", 1, "t", rank, "2026-11-22T00:00:00Z").priority,
    );
    expect(order.every((p) => scale.includes(p))).toBe(true);
    expect(order).toEqual([...order].sort((a, b) => a - b));
    expect(triagePacket("o/r", "q", 1, "x".repeat(400), "none", "t").title.length).toBe(180);

    // The issue exists either way: a failed enqueue is reported, not hidden and not thrown.
    const failing = recorder(answers({ code: 1, stderr: "error: store unreachable\nrepo-queue: enqueue failed" }));
    expect(await new Ops(CFG, failing.run).fileIdea("t", "", "none")).toEqual({
      issue: 12400,
      url: "https://x/12400",
      queued: false,
      note: "filed, not queued: repo-queue: enqueue failed",
    });
    expect(JSON.parse(failing.calls[0].stdin!).labels).toEqual([]);
    const refused = recorder();
    await expect(new Ops(CFG, refused.run).fileIdea("t", "", "urgent")).rejects.toThrow(/rank/);
    await expect(new Ops(CFG, refused.run).fileIdea("  ", "", "none")).rejects.toThrow(OpError);
    expect(refused.calls.length).toBe(0);
  });

  test("a stopped unit that systemd has unloaded is still listed", async () => {
    const r = recorder((argv) =>
      argv[2] === "list-units"
        ? "  pm-aggregator.service loaded active running PM dashboard\n● pm-old.service not-found inactive dead pm-old.service\n"
        : [
            "pm-aggregator.service enabled enabled",
            "pm-console.service    disabled enabled",
            "pm-git-sync.timer     enabled  enabled",
            "fleet-worker@.service static   -",
            "sshd.service          enabled  enabled",
            "",
            "5 unit files listed.",
          ].join("\n"),
    );
    const units = await new Ops(CFG, r.run).services();
    expect(units.map((u) => [u.unit, u.active, u.load])).toEqual([
      ["pm-aggregator.service", "active", "loaded"],
      ["pm-console.service", "inactive", "not-loaded"],
      ["pm-git-sync.timer", "inactive", "not-loaded"],
      ["pm-old.service", "inactive", "not-found"],
    ]);
    expect(r.calls.map((c) => c.argv.slice(0, 3))).toEqual([
      ["systemctl", "--user", "list-units"],
      ["systemctl", "--user", "list-unit-files"],
    ]);
    expect(parseUnitFiles("a b\n\nx.service\n pm-a.service  masked\n")).toEqual([
      { unit: "pm-a.service", state: "masked" },
    ]);
    // The loaded list is the one that must work; the file list only adds to it.
    const partial = recorder((argv) =>
      argv[2] === "list-units" ? "pm-a.service loaded active running A\n" : { code: 1, stderr: "failed" },
    );
    expect((await new Ops(CFG, partial.run).services()).map((u) => u.unit)).toEqual(["pm-a.service"]);
  });

  test("services: only allowlisted units, and the interface's own units cannot be stopped", async () => {
    const r = recorder();
    const ops = new Ops(CFG, r.run);
    await ops.serviceAct("pm-aggregator.service", "restart");
    await ops.serviceAct("pm-tunnel.service", "restart");
    expect(r.calls.map((c) => c.argv)).toEqual([
      ["systemctl", "--user", "restart", "pm-aggregator.service"],
      ["systemctl", "--user", "restart", "pm-tunnel.service"],
    ]);
    await expect(ops.serviceAct("pm-tunnel.service", "stop")).rejects.toThrow(/not stopped/);
    await expect(ops.serviceAct("pm-console.service", "stop")).rejects.toThrow(/not stopped/);
    for (const [unit, action] of [
      ["sshd.service", "restart"],
      ["pm-x.service; reboot", "restart"],
      ["pm-x.service", "kill"],
      ["pm-x.service", "daemon-reload"],
      ["pm-x", "restart"],
      ["../pm-x.service", "restart"],
      ["--now", "stop"],
      [7, "stop"],
    ]) {
      await expect(ops.serviceAct(unit, action)).rejects.toThrow(OpError);
    }
    await expect(ops.serviceLogs("dbus.service", "10")).rejects.toThrow(OpError);
    await ops.serviceLogs("opencode.service", "9999");
    expect(r.calls.length).toBe(3);
    expect(r.calls[2].argv).toEqual([
      "journalctl",
      "--user",
      "-u",
      "opencode.service",
      "-n",
      "400",
      "--no-pager",
      "-o",
      "short-iso",
    ]);
  });

  test("property: no generated unit name outside the allowlist is manageable", () => {
    const next = rng(4872);
    const pieces = [
      "pm-",
      "opencode",
      "fleet-",
      "sshd",
      "dbus",
      "x",
      ".service",
      ".timer",
      ".socket",
      ";",
      " ",
      "/",
      "..",
      "$(",
      "*",
      "-",
      "@",
      "\n",
      "queue-broker",
    ];
    for (let i = 0; i < 3000; i++) {
      const name = Array.from(
        { length: 1 + Math.floor(next() * 4) },
        () => pieces[Math.floor(next() * pieces.length)],
      ).join("");
      const allowed = manageable(name, CFG.unitPatterns);
      const plain = /^[A-Za-z0-9@._-]+\.(service|timer)$/.test(name);
      const prefixed = ["pm-", "opencode", "openchamber", "queue-broker", "secrets-broker", "fleet-"].some((p) =>
        name.startsWith(p),
      );
      expect(allowed).toBe(plain && prefixed);
    }
  });

  test("property: the unit parser reads back what systemctl prints", () => {
    const next = rng(20261122);
    const word = () =>
      ["active", "inactive", "failed", "running", "dead", "waiting", "exited", "loaded", "not-found"][
        Math.floor(next() * 9)
      ];
    for (let i = 0; i < 300; i++) {
      const units = Array.from({ length: Math.floor(next() * 6) }, (_, n) => ({
        unit: `pm-u${i}-${n}.${next() < 0.5 ? "service" : "timer"}`,
        load: word(),
        active: word(),
        sub: word(),
        description: next() < 0.3 ? "" : `Some unit — number ${n}`,
      }));
      const printed = units
        .map(
          (u) =>
            `${next() < 0.2 ? "● " : "  "}${u.unit}${" ".repeat(1 + Math.floor(next() * 9))}${u.load} ${u.active}   ${u.sub} ${u.description}`,
        )
        .join("\n");
      expect(parseUnits(`${printed}\n\n`)).toEqual(units);
    }
    expect(parseUnits("0 loaded units listed.\n")).toEqual([]);
  });

  test("merging and a failing command", async () => {
    const r = recorder((argv) =>
      argv.includes("-X")
        ? JSON.stringify({ merged: true, sha: "abc" })
        : { code: 1, stderr: "gh: Not Found (HTTP 404)" },
    );
    const ops = new Ops(CFG, r.run);
    expect(await ops.mergePr("12240")).toEqual({ number: 12240, merged: true, sha: "abc" });
    expect(r.calls[0].argv).toEqual([
      "fleet",
      "gh",
      "api",
      "-X",
      "PUT",
      "repos/developing-today/code/pulls/12240/merge",
      "-f",
      "merge_method=merge",
    ]);
    await expect(ops.pr("12240")).rejects.toThrow(/failed \(exit 1\).*Not Found/);
    await expect(ops.mergePr("12240/merge")).rejects.toThrow(OpError);
    // With the commit the owner was shown, GitHub is told to merge that
    // and nothing pushed since.
    const head = "0123456789abcdef0123456789abcdef01234567";
    await ops.mergePr(12240, head);
    expect(r.calls.at(-1)?.argv.slice(-4)).toEqual(["-f", "merge_method=merge", "-f", `sha=${head}`]);
    // What is not a commit's full name is not passed on as one.
    const before = r.calls.length;
    await expect(ops.mergePr(12240, "main")).rejects.toThrow(/head: a commit/);
    await expect(ops.mergePr(12240, 7)).rejects.toThrow(/head: a commit/);
    expect(r.calls.length).toBe(before);
    // And none given is none asked for.
    await ops.mergePr(12240, null);
    expect(r.calls.at(-1)?.argv.slice(-2)).toEqual(["-f", "merge_method=merge"]);
  });
});

describe("HTTP surface", () => {
  type Sent = { url: string; method: string; headers: Record<string, string>; body: string | null; redirect?: string };
  function deps(
    who: "owner" | 401 | 403,
    r = recorder(),
    answer: (url: string) => Response = () => new Response("dash", { headers: { "content-type": "text/html" } }),
  ): Deps & { audited: Record<string, unknown>[]; sent: Sent[] } {
    const audited: Record<string, unknown>[] = [];
    const sent: Sent[] = [];
    return {
      sent,
      upstream: async (url, init) => {
        sent.push({
          url,
          method: String(init.method),
          headers: Object.fromEntries(new Headers(init.headers)),
          body: init.body ? new TextDecoder().decode(init.body as ArrayBuffer) : null,
          redirect: init.redirect,
        });
        return answer(url);
      },
      cfg: CFG,
      ops: new Ops(CFG, r.run),
      identify: async () => {
        if (who === "owner") return { via: "cloudflare-access", email: OWNER };
        throw new AuthError(who, who === 401 ? "sign-in required" : "this identity is not the owner");
      },
      audit: async (e) => {
        audited.push(e);
      },
      audited,
    };
  }
  const get = (path: string) => new Request(`https://pm.security.cab${path}`, { headers: { host: "pm.security.cab" } });
  const post = (
    path: string,
    body: unknown,
    headers: Record<string, string> = { "x-pm-console": "1", origin: "https://pm.security.cab" },
  ) =>
    new Request(`https://pm.security.cab${path}`, {
      method: "POST",
      body: JSON.stringify(body),
      headers: { host: "pm.security.cab", "content-type": "application/json", ...headers },
    });

  test("only the asset links and the health check answer without sign-in", async () => {
    const d = deps(401);
    const links = await handle(get("/.well-known/assetlinks.json"), d);
    expect(links.status).toBe(200);
    expect(await links.json()).toEqual(assetLinks(CFG));
    expect(assetLinks(CFG)[0]).toMatchObject({ target: { package_name: "today.developing.pmconsole" } });
    expect(assetLinks({ androidPackage: "p", androidFingerprints: [] })).toEqual([]);
    expect(await (await handle(get("/console/health"), d)).json()).toEqual({ ok: true });
    for (const path of [
      "/console/",
      "/console/app.js",
      "/console/api/me",
      "/console/api/services",
      "/console/api/queue",
    ]) {
      expect((await handle(get(path), d)).status).toBe(401);
    }
    expect((await handle(get("/console/api/me"), deps(403))).status).toBe(403);
    expect((await handle(get("/elsewhere"), d)).status).toBe(401);
    expect(d.sent).toEqual([]);
    // With no dashboard configured there is nothing outside /console.
    const alone = { ...deps("owner"), cfg: readConfig({ HOME: "/h", PM_CONSOLE_DASHBOARD_URL: "" }) };
    expect((await handle(get("/elsewhere"), alone)).status).toBe(404);
  });

  test("every file the page and the worker load is served here, served by the edge, and shipped in the edge's build", async () => {
    // Three lists name the app's files: this service's, the edge's, and
    // what the edge's build copies. A file missing from one of them
    // works on the laptop and is a 404 behind Cloudflare, or the other
    // way round; and the worker cannot open the app offline without
    // every one of them.
    const read = (path: string) => readFileSync(join(import.meta.dir, "..", path), "utf8");
    const page = [...read("public/index.html").matchAll(/(?:src|href)="\/console\/([^"]+)"/g)].map((m) => m[1]);
    const worker = [
      ...(/const FILES = \[([^\]]+)\]/.exec(read("public/sw.js"))?.[1] ?? "").matchAll(/"\/console\/([^"]+)"/g),
    ].map((m) => m[1]);
    const loaded = [...new Set([...page, ...worker, "sw.js"])].sort();
    expect(loaded).toEqual(["app.css", "app.js", "icon.svg", "outbox.js", "sw.js"]);
    // The worker keeps everything the page loads, or the page would not open offline.
    expect([...new Set(worker)].sort()).toEqual(page.sort());
    const d = deps("owner");
    for (const name of loaded) expect([name, (await handle(get(`/console/${name}`), d)).status]).toEqual([name, 200]);
    const edgeServes = [
      ...(/const STATIC: Record<string, string> = \{([^}]+)\}/.exec(read("edge/src/edge.ts"))?.[1] ?? "").matchAll(
        /"([^"]+)":/g,
      ),
    ].map((m) => m[1]);
    expect(edgeServes.sort()).toEqual(loaded);
    const built = [
      ...(
        /for \(const name of \[([^\]]+)\]\) \{\s*cpSync\(join\(here, "\.\.", "public"/.exec(
          read("edge/build.mjs"),
        )?.[1] ?? ""
      ).matchAll(/"([^"]+)"/g),
    ].map((m) => m[1]);
    expect(built.sort()).toEqual([...loaded, "index.html"].sort());
  });

  test("the page and its files are served to the owner with a strict policy", async () => {
    const d = deps("owner");
    const page = await handle(get("/console/"), d);
    expect(page.headers.get("content-type")).toContain("text/html");
    expect(page.headers.get("content-security-policy")).toContain("default-src 'self'");
    const html = await page.text();
    expect(html).toContain('src="/console/app.js"');
    expect((await handle(get("/console/app.css"), d)).headers.get("content-type")).toContain("text/css");
    // The file that keeps what the owner did with no connection comes
    // before the app, which uses it; and the worker loads it by this address.
    expect(html.indexOf('src="/console/outbox.js"')).toBeGreaterThan(0);
    expect(html.indexOf('src="/console/outbox.js"')).toBeLessThan(html.indexOf('src="/console/app.js"'));
    const outbox = await handle(get("/console/outbox.js"), d);
    expect([outbox.status, outbox.headers.get("content-type")]).toEqual([200, "text/javascript; charset=utf-8"]);
    expect(await outbox.text()).toContain("PMOutbox");
    expect(await (await handle(get("/console/sw.js"), d)).text()).toContain('importScripts("/console/outbox.js")');
    // The site is only ever HTTPS, and says so.
    expect(page.headers.get("strict-transport-security")).toBe("max-age=31536000");
    // A visit does not download the app again: each file says which
    // version of itself it is, and a browser that names it is sent nothing.
    const style = await handle(get("/console/app.css"), d);
    const tag = style.headers.get("etag") as string;
    expect(tag).toMatch(/^"[A-Za-z0-9_-]{22}"$/);
    expect(tag).not.toBe(page.headers.get("etag")); // a file's tag is of its own content
    expect((await handle(get("/console/app.css"), d)).headers.get("etag")).toBe(tag); // and does not change by itself
    const asked = (named: string) =>
      handle(
        new Request("https://pm.security.cab/console/app.css", {
          headers: { host: "pm.security.cab", "if-none-match": named },
        }),
        d,
      );
    for (const named of [tag, `W/${tag}`, `"other", ${tag}`]) {
      const still = await asked(named);
      expect([named, still.status, await still.text()]).toEqual([named, 304, ""]);
      expect(still.headers.get("etag")).toBe(tag);
    }
    const sent = await asked('"an-older-version"');
    expect(sent.status).toBe(200);
    expect(await sent.text()).toBe(await style.text());
    expect((await handle(get("/console"), d)).status).toBe(308);
    // A path that climbs out of /console is the dashboard's to answer, never a file of this service.
    expect(await (await handle(get("/console/../src/server.ts"), d)).text()).toBe("dash");
    expect(d.sent.map((c) => c.url)).toEqual(["http://127.0.0.1:4870/src/server.ts"]);
    expect((await handle(get("/console/config.ts"), d)).status).toBe(404);
    // Names every object has are not files, and a path that does not decode is a bad request.
    for (const name of ["constructor", "__proto__", "toString", "hasOwnProperty"]) {
      expect((await handle(get(`/console/${name}`), d)).status).toBe(404);
    }
    expect((await handle(get("/console/api/prs/%E0%A4%A"), d)).status).toBe(400);
    expect((await handle(post("/console/api/queue/%ZZ/unblock", { reason: "x" }), d)).status).toBe(400);
    expect(await (await handle(get("/console/api/me"), d)).json()).toMatchObject({
      email: OWNER,
      via: "cloudflare-access",
      repo: "developing-today/code",
    });
  });

  test("a write needs the console's header and its own origin, and is audited", async () => {
    const r = recorder();
    const d = deps("owner", r);
    expect((await handle(post("/console/api/services/pm-aggregator.service/restart", {}, {}), d)).status).toBe(403);
    expect(
      (
        await handle(
          post(
            "/console/api/services/pm-aggregator.service/restart",
            {},
            { "x-pm-console": "1", origin: "https://evil.example" },
          ),
          d,
        )
      ).status,
    ).toBe(403);
    expect(r.calls.length).toBe(0);
    const ok = await handle(post("/console/api/services/pm-aggregator.service/restart", {}), d);
    expect(await ok.json()).toEqual({ ok: true, result: { unit: "pm-aggregator.service", action: "restart" } });
    expect(r.calls.map((c) => c.argv)).toEqual([["systemctl", "--user", "restart", "pm-aggregator.service"]]);
    const refused = await handle(post("/console/api/services/pm-tunnel.service/stop", {}), d);
    expect(refused.status).toBe(409);
    const bad = await handle(
      new Request("https://pm.security.cab/console/api/messages", {
        method: "POST",
        body: "[1]",
        headers: { host: "pm.security.cab", "x-pm-console": "1" },
      }),
      d,
    );
    expect(bad.status).toBe(400);
    // Each action is recorded before it runs and again with its outcome.
    expect(d.audited.map((e) => [e.action, e.phase, e.ok, e.who])).toEqual([
      ["service.restart", "start", undefined, OWNER],
      ["service.restart", "done", true, OWNER],
      ["service.stop", "start", undefined, OWNER],
      ["service.stop", "done", false, OWNER],
    ]);
    expect(
      (
        await handle(
          new Request("https://pm.security.cab/console/api/me", {
            method: "DELETE",
            headers: { host: "pm.security.cab" },
          }),
          d,
        )
      ).status,
    ).toBe(405);
  });

  const PUBLIC = [
    "/icons/icon-192.png",
    "/icons/icon-512.png",
    "/icons/icon-maskable-512.png",
    "/icons/apple-touch-icon.png",
  ];

  test("the dashboard is reached only by the owner, apart from its install files", async () => {
    const out = deps(401);
    for (const path of [
      "/",
      "/attention",
      "/api/board",
      "/sw.js",
      "/icons/",
      "/icons/evil.png",
      "/icons/../attention",
      "/icons/%2e%2e/attention",
      "/icons/..%2fattention",
      "/icons/icon-192.png/../../attention",
      "/icons/icon-192.png/",
      "/icons/icon-192.png%00",
      "/ICONS/icon-192.png",
      "//icons/icon-192.png",
      "/manifest.webmanifest/x",
      "/manifest.webmanifest;x",
      "/x/manifest.webmanifest",
      "/.well-known/assetlinks.json/x",
    ]) {
      expect([path, (await handle(get(path), out)).status]).toEqual([path, 401]);
    }
    expect(out.sent).toEqual([]);
    expect((await handle(get("/attention"), deps(403))).status).toBe(403);

    // The install files: asked for exactly, read-only, and nothing the caller sent goes along.
    for (const path of PUBLIC) {
      const res = await handle(
        new Request(`https://pm.security.cab${path}?v=2`, {
          headers: { host: "pm.security.cab", cookie: "CF_Authorization=x", authorization: "Bearer y" },
        }),
        out,
      );
      expect(await res.text()).toBe("dash");
    }
    expect(out.sent.map((c) => c.url)).toEqual(PUBLIC.map((p) => `http://127.0.0.1:4870${p}?v=2`));
    for (const call of out.sent) {
      expect(call.method).toBe("GET");
      expect(call.headers).toEqual({ host: "pm.security.cab", "accept-encoding": "identity" });
    }
    expect((await handle(post("/icons/icon-192.png", {}), out)).status).toBe(401);
    // The install manifest is the app's own: public, and it opens the app.
    const manifest = await handle(get("/manifest.webmanifest"), out);
    expect(manifest.headers.get("content-type")).toContain("application/manifest+json");
    expect(await manifest.json()).toMatchObject({
      id: "/console/",
      start_url: "/console/",
      scope: "/",
      display: "standalone",
    });
    expect((await handle(post("/manifest.webmanifest", {}), out)).status).toBe(401);
    expect(out.sent.length).toBe(PUBLIC.length);
  });

  test("property: no spelling of a path reaches the dashboard without sign-in unless it is an install file", async () => {
    const next = rng(4870);
    const pieces = [
      "/",
      "icons",
      "icon-192.png",
      "manifest.webmanifest",
      "attention",
      "..",
      ".",
      "%2e%2e",
      "%2E",
      "%2f",
      "%5c",
      "\\",
      "%00",
      ";",
      "?",
      "#",
      "ICONS",
      "api",
      ".well-known",
      "assetlinks.json",
      "console",
      " ",
      "%20",
    ];
    const out = deps(401);
    const allowed = new Set(PUBLIC.map((p) => `http://127.0.0.1:4870${p}`));
    let reached = 0;
    for (let i = 0; i < 4000; i++) {
      const path = `/${Array.from({ length: 1 + Math.floor(next() * 7) }, () => pieces[Math.floor(next() * pieces.length)]).join(next() < 0.7 ? "/" : "")}`;
      let req: Request;
      try {
        req = get(path);
      } catch {
        continue; // not a request a client could send
      }
      const before = out.sent.length;
      const res = await handle(req, out);
      if (out.sent.length > before) {
        reached++;
        expect(allowed.has(out.sent[before].url.split("?")[0])).toBe(true);
      } else if (!new URL(req.url).pathname.startsWith("/.well-known/assetlinks.json")) {
        expect([path, res.status === 401 || res.status === 200 || res.status === 308]).toEqual([path, true]);
      }
    }
    expect(reached).toBeGreaterThan(0); // the generator does produce the public files
    for (const call of out.sent) expect(allowed.has(call.url.split("?")[0])).toBe(true);
  });

  test("the owner's request passes to the dashboard as it was checked, and the answer comes back", async () => {
    const d = deps("owner", recorder(), (url) =>
      url.endsWith("/moved")
        ? new Response(null, { status: 302, headers: { location: "/attention" } })
        : new Response("<p>board</p>", {
            status: 207,
            headers: {
              "content-type": "text/html",
              "content-encoding": "gzip",
              "content-length": "999",
              connection: "close",
              "set-cookie": "a=1",
              "x-board": "yes",
            },
          }),
    );
    const res = await handle(
      new Request("https://pm.security.cab/attention/../board?lane=id&x=%2F", {
        headers: {
          host: "pm.security.cab",
          cookie: "CF_Authorization=t",
          "cf-access-jwt-assertion": "t",
          connection: "keep-alive, upgrade",
          upgrade: "websocket",
          "accept-encoding": "br",
          "x-forwarded-for": "203.0.113.9",
        },
      }),
      d,
    );
    expect(res.status).toBe(207);
    expect(await res.text()).toBe("<p>board</p>");
    expect(res.headers.get("x-board")).toBe("yes");
    expect(res.headers.get("set-cookie")).toBe("a=1");
    expect(res.headers.get("content-encoding")).toBeNull();
    expect(res.headers.get("connection")).toBeNull();
    expect(d.sent).toEqual([
      {
        url: "http://127.0.0.1:4870/board?lane=id&x=%2F",
        method: "GET",
        headers: {
          host: "pm.security.cab",
          cookie: "CF_Authorization=t",
          "cf-access-jwt-assertion": "t",
          "accept-encoding": "identity",
          "x-forwarded-for": "203.0.113.9",
        },
        body: null,
        redirect: "manual",
      },
    ]);

    // A redirect is the browser's to follow, not the console's.
    const moved = await handle(get("/moved"), d);
    expect([moved.status, moved.headers.get("location")]).toEqual([302, "/attention"]);

    // Writes: this site's own pass with their body; another site's do not reach the dashboard.
    const before = d.sent.length;
    const own = await handle(post("/api/cards/7/move", { to: "done" }), d);
    expect(own.status).toBe(207);
    expect(d.sent[before]).toMatchObject({
      url: "http://127.0.0.1:4870/api/cards/7/move",
      method: "POST",
      body: '{"to":"done"}',
    });
    expect(d.sent[before].headers["content-length"]).toBeUndefined();
    for (const headers of [
      { origin: "https://evil.example" },
      { origin: "null" },
      { origin: "https://pm.security.cab", "sec-fetch-site": "cross-site" },
    ] as Record<string, string>[]) {
      expect((await handle(post("/api/cards/7/move", { to: "done" }, headers), d)).status).toBe(403);
    }
    expect(d.sent.length).toBe(before + 1);
    // An agent on the host, with no browser headers at all, still writes.
    expect((await handle(post("/api/cards/7/move", {}, {}), d)).status).toBe(207);

    const down = deps("owner");
    down.upstream = async () => {
      throw new Error("connection refused");
    };
    const failed = await handle(get("/attention"), down);
    expect([failed.status, await failed.json()]).toEqual([502, { error: "the dashboard did not answer" }]);
  });

  test("the Android app is handed to the signed-in owner only, when a build is on the host", async () => {
    const dir = mkdtempSync(join(tmpdir(), "pm-console-test-"));
    try {
      const cfg = readConfig({ HOME: "/h", PM_CONSOLE_ANDROID_APK: join(dir, "pm-console.apk") });
      expect(readConfig({ HOME: "/h" }).androidApk).toBe("/h/.local/state/pm-console/android/pm-console.apk");
      const owner = { ...deps("owner"), cfg };
      // No build yet: the page is told so, and the link answers 404.
      expect(((await (await handle(get("/console/api/me"), owner)).json()) as { androidApp: boolean }).androidApp).toBe(
        false,
      );
      expect((await handle(get("/console/pm-console.apk"), owner)).status).toBe(404);
      writeFileSync(join(dir, "pm-console.apk"), "PK\u0003\u0004 signed apk bytes");
      expect(((await (await handle(get("/console/api/me"), owner)).json()) as { androidApp: boolean }).androidApp).toBe(
        true,
      );
      const res = await handle(get("/console/pm-console.apk"), owner);
      expect(res.status).toBe(200);
      expect(res.headers.get("content-type")).toBe("application/vnd.android.package-archive");
      expect(res.headers.get("content-disposition")).toBe('attachment; filename="pm-console.apk"');
      expect(await res.text()).toBe("PK\u0003\u0004 signed apk bytes");
      // Not for anyone else, and not by another method or name.
      expect((await handle(get("/console/pm-console.apk"), { ...deps(401), cfg })).status).toBe(401);
      expect((await handle(get("/console/pm-console.apk"), { ...deps(403), cfg })).status).toBe(403);
      expect((await handle(post("/console/pm-console.apk", {}), owner)).status).toBe(404);
      expect(owner.sent).toEqual([]); // never passed to the dashboard
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("an action that cannot be recorded is not taken; one already taken is reported as it went", async () => {
    const r = recorder();
    const order: string[] = [];
    const d = deps("owner", r);
    const run = r.run;
    d.ops = new Ops(CFG, async (argv, opts) => {
      order.push("run");
      return run(argv, opts);
    });
    d.audit = async (e) => {
      order.push(`audit:${e.phase}`);
    };
    expect((await handle(post("/console/api/services/pm-aggregator.service/restart", {}), d)).status).toBe(200);
    expect(order).toEqual(["audit:start", "run", "audit:done"]);

    // The record cannot be written: nothing runs.
    d.audit = async () => {
      throw new Error("disk full");
    };
    const refused = await handle(post("/console/api/prs/12240/merge", {}), d);
    expect(refused.status).toBe(502);
    expect(((await refused.json()) as { error: string }).error).toContain("not taken");
    expect(r.calls.length).toBe(1);

    // The record of the outcome cannot be written: the merge happened, and
    // the answer says so, or the owner would press Merge again.
    let writes = 0;
    d.audit = async () => {
      if (++writes > 1) throw new Error("disk full");
    };
    d.ops = new Ops(CFG, async () => ({ code: 0, stdout: JSON.stringify({ merged: true, sha: "abc" }), stderr: "" }));
    const merged = await handle(post("/console/api/prs/12240/merge", {}), d);
    expect(merged.status).toBe(200);
    expect(await merged.json()).toEqual({ ok: true, result: { number: 12240, merged: true, sha: "abc" } });
    expect(writes).toBe(2);
  });

  test("the server hands the connection's address to sign-in", async () => {
    const d = deps("owner");
    const seen: (string | null)[] = [];
    d.identify = async (_req, peer) => {
      seen.push(peer);
      return { via: "loopback", email: "local" };
    };
    await handle(get("/console/api/me"), d, "203.0.113.9");
    await handle(get("/console/api/me"), d);
    expect(seen).toEqual(["203.0.113.9", null]);
  });

  test("each write route reaches its operation", async () => {
    const r = recorder((argv) =>
      argv[1] === "gh" ? JSON.stringify({ number: 1, html_url: "u", merged: true, sha: "s", labels: [] }) : "",
    );
    const d = deps("owner", r);
    const run = async (path: string, body: unknown) => (await handle(post(path, body), d)).status;
    expect(await run("/console/api/queue/20306/unblock", { reason: "blocker closed" })).toBe(200);
    expect(await run("/console/api/queue/20306/cancel", { reason: "duplicate" })).toBe(200);
    expect(await run("/console/api/priorities/12279/rank", { rank: "now" })).toBe(200);
    expect(await run("/console/api/issues/12279/comment", { body: "go" })).toBe(200);
    expect(await run("/console/api/ideas", { title: "an idea", body: "", rank: "next" })).toBe(200);
    expect(await run("/console/api/messages", { to: ["all"], text: "hello", command: false })).toBe(200);
    expect(await run("/console/api/prs/12240/merge", {})).toBe(200);
    expect(await run("/console/api/queue/20306/claim", { reason: "x" })).toBe(400);
    expect(await run("/console/api/nothing", {})).toBe(404);
    expect(d.audited.filter((e) => e.phase === "start").length).toBe(8); // the refused queue action was recorded too
    expect(d.audited.filter((e) => e.ok).map((e) => e.action)).toEqual([
      "queue.unblock",
      "queue.cancel",
      "priority.rank",
      "issue.comment",
      "idea.file",
      "message.send",
      "pr.merge",
    ]);
    expect(JSON.stringify(d.audited)).not.toContain("hello"); // message text is not copied into the audit log
  });
});
