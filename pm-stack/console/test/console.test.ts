// pm-console: who gets in, what each operation runs, and what the HTTP
// surface refuses.
//
// Oracles: tokens are signed here with keys generated here, so "valid"
// is defined by the test and not by the code under test; every
// operation runs against a recording runner, so what would reach the
// machine is compared as an exact argument vector; and seeded generators
// drive the properties (any changed token byte is refused; no generated
// unit name outside the allowlist reaches systemctl; the unit parser
// reads back what a formatter wrote).
import { beforeEach, describe, expect, test } from "bun:test";
import { AuthError, authorize, cameThroughEdge, clearJwksCache, identify, verifyJwt } from "../src/auth.ts";
import { type Config, ConfigError, type Issuer, readConfig } from "../src/config.ts";
import { manageable, OpError, Ops, parseUnits, type Run } from "../src/ops.ts";
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
const OWNER = "drewrypope@gmail.com";

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

function keyServer(sets: Record<string, JsonWebKey[]>) {
  const calls: string[] = [];
  const fetchImpl = async (url: string) => {
    calls.push(url);
    return { ok: url in sets, status: url in sets ? 200 : 404, json: async () => ({ keys: sets[url] ?? [] }) };
  };
  return { fetchImpl, calls };
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

  test("fetches a rotated key once, and caches the key set otherwise", async () => {
    const rotated = await signer("RS256", "k9");
    const sets: Record<string, JsonWebKey[]> = { [ACCESS.jwksUri]: [rsa.jwk] };
    const keys = keyServer(sets);
    await verifyJwt(await rsa.sign(good(ACCESS)), ACCESS, keys.fetchImpl, NOW);
    await verifyJwt(await rsa.sign(good(ACCESS)), ACCESS, keys.fetchImpl, NOW + 1000);
    expect(keys.calls.length).toBe(1);
    sets[ACCESS.jwksUri] = [rsa.jwk, rotated.jwk];
    await verifyJwt(await rotated.sign(good(ACCESS)), ACCESS, keys.fetchImpl, NOW + 2000);
    expect(keys.calls.length).toBe(2);
  });
});

describe("who is the owner", () => {
  const cfg = { owners: [OWNER], certNames: ["laptop.devices.central"], issuers: [ACCESS, CENTRAL] };
  const edge = (headers: Record<string, string> = {}) =>
    new Request("https://pm.security.cab/console/api/me", {
      headers: { host: "pm.security.cab", "cf-ray": "1", ...headers },
    });

  test("the owner's verified address gets in through either issuer", async () => {
    const keys = keyServer({ [ACCESS.jwksUri]: [rsa.jwk], [CENTRAL.jwksUri]: [ec.jwk] });
    expect(
      await identify(edge({ "cf-access-jwt-assertion": await rsa.sign(good(ACCESS)) }), cfg, keys.fetchImpl, NOW),
    ).toEqual({ via: "cloudflare-access", email: OWNER, sub: "user-1" });
    expect(
      await identify(
        edge({ cookie: `x=1; CF_Authorization=${await rsa.sign(good(ACCESS, { email: "DrewryPope@Gmail.com" }))}` }),
        cfg,
        keys.fetchImpl,
        NOW,
      ),
    ).toMatchObject({ email: OWNER });
    expect(
      await identify(edge({ authorization: `Bearer ${await ec.sign(good(CENTRAL))}` }), cfg, keys.fetchImpl, NOW),
    ).toMatchObject({ via: "central", email: OWNER });
  });

  test("anyone else is refused, by status", async () => {
    const keys = keyServer({ [ACCESS.jwksUri]: [rsa.jwk], [CENTRAL.jwksUri]: [ec.jwk] });
    const status = async (req: Request) =>
      identify(req, cfg, keys.fetchImpl, NOW).then(
        () => 200,
        (e: AuthError) => e.status,
      );
    expect(await status(edge())).toBe(401);
    expect(await status(edge({ "cf-access-jwt-assertion": "garbage" }))).toBe(401);
    expect(await status(edge({ "cf-access-jwt-assertion": await other.sign(good(ACCESS)) }))).toBe(401);
    expect(
      await status(edge({ "cf-access-jwt-assertion": await rsa.sign(good(ACCESS, { email: "someone@gmail.com" })) })),
    ).toBe(403);
    expect(
      await status(edge({ "cf-access-jwt-assertion": await rsa.sign(good(ACCESS, { email_verified: false })) })),
    ).toBe(403);
    // A token for one issuer is not accepted in another issuer's place.
    expect(await status(edge({ authorization: `Bearer ${await rsa.sign(good(ACCESS))}` }))).toBe(401);
    // With no owners configured, a valid token still opens nothing.
    const closed = { ...cfg, owners: [], certNames: [] };
    await expect(
      identify(edge({ "cf-access-jwt-assertion": await rsa.sign(good(ACCESS)) }), closed, keys.fetchImpl, NOW),
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

  test("only a request that crossed no proxy is local", async () => {
    const local = (headers: Record<string, string>) => new Request("http://127.0.0.1:4872/console/api/me", { headers });
    expect(await identify(local({ host: "127.0.0.1:4872" }), cfg)).toEqual({ via: "loopback", email: "local" });
    expect(cameThroughEdge(local({ host: "localhost:4872" }))).toBe(false);
    for (const headers of [
      { host: "pm.security.cab" },
      { host: "evil.example:4872" }, // a rebound name resolving to loopback
      { host: "127.0.0.1:4872", "cf-connecting-ip": "203.0.113.9" },
      { host: "127.0.0.1:4872", "x-forwarded-for": "203.0.113.9" },
      { host: "127.0.0.1:4872", "cdn-loop": "cloudflare" },
    ]) {
      await expect(identify(local(headers), cfg)).rejects.toThrow(/sign-in required/);
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
        PM_CONSOLE_OWNER_EMAILS: " DrewryPope@gmail.com ,",
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

  test("the queue is read with bound parameters and never written", async () => {
    const r = recorder(() => "secret-token\n");
    const sent: { url: string; body: { requests: { stmt?: { sql: string; args: unknown[] } }[] }; auth: string }[] = [];
    const ops = new Ops(CFG, r.run, async (url, init) => {
      sent.push({ url, body: JSON.parse(init.body), auth: init.headers.Authorization });
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

  test("ranking an issue removes the other ranks and adds the new one", async () => {
    const r = recorder((argv) =>
      argv.includes("repos/developing-today/code/issues/12279")
        ? JSON.stringify({ labels: [{ name: "now" }, { name: "app/id" }, { name: "priority/high" }] })
        : "{}",
    );
    const ops = new Ops(CFG, r.run);
    await ops.setRank("12279", "next");
    expect(r.calls.map((c) => c.argv.slice(3))).toEqual([
      ["repos/developing-today/code/issues/12279"],
      ["-X", "DELETE", "repos/developing-today/code/issues/12279/labels/now"],
      ["-X", "DELETE", "repos/developing-today/code/issues/12279/labels/priority%2Fhigh"],
      ["-X", "POST", "repos/developing-today/code/issues/12279/labels", "--input", "-"],
    ]);
    expect(r.calls[3].stdin).toBe('{"labels":["next"]}');
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
    expect((await new Ops(CFG, r.run).priorities()).map((i) => [i.number, i.rank])).toEqual([
      [7, "expedite"],
      [1, "now"],
      [9, "now"],
      [2, "priority/high"],
      [5, "next"],
    ]);
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
  });
});

describe("HTTP surface", () => {
  function deps(who: "owner" | 401 | 403, r = recorder()): Deps & { audited: Record<string, unknown>[] } {
    const audited: Record<string, unknown>[] = [];
    return {
      cfg: CFG,
      ops: new Ops(CFG, r.run),
      identify: async () => {
        if (who === "owner") return { via: "cloudflare-access", email: OWNER };
        throw new AuthError(who, who === 401 ? "sign-in required" : "this identity is not the owner");
      },
      audit: (e) => audited.push(e),
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
    expect((await handle(get("/elsewhere"), d)).status).toBe(404);
  });

  test("the page and its files are served to the owner with a strict policy", async () => {
    const d = deps("owner");
    const page = await handle(get("/console/"), d);
    expect(page.headers.get("content-type")).toContain("text/html");
    expect(page.headers.get("content-security-policy")).toContain("default-src 'self'");
    expect(await page.text()).toContain('src="/console/app.js"');
    expect((await handle(get("/console/app.css"), d)).headers.get("content-type")).toContain("text/css");
    expect((await handle(get("/console"), d)).status).toBe(308);
    expect((await handle(get("/console/../src/server.ts"), d)).status).toBe(404);
    expect((await handle(get("/console/config.ts"), d)).status).toBe(404);
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
    expect(d.audited.map((e) => [e.action, e.ok, e.who])).toEqual([
      ["service.restart", true, OWNER],
      ["service.stop", false, OWNER],
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
