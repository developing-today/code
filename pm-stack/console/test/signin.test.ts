// Sign-in settings through the console: reading which methods are on,
// and handing a Google client to the setup script.
//
// Stand-in: the setup script (scripts/expose.py, tested on its own in
// scripts/test_expose.py). Everything else is the console's own code.
// The point of most checks here is where the client's two values go:
// to the script's standard input, and nowhere else.
import { describe, expect, test } from "bun:test";
import { actions, perform } from "../src/actions.ts";
import { readConfig } from "../src/config.ts";
import { Docs } from "../src/docs.ts";
import { HOLDABLE } from "../src/edge-proto.ts";
import { OpError, Ops, type RunResult } from "../src/ops.ts";
import { Work } from "../src/work.ts";

const CLIENT = "123456789012-abc123def456.apps.googleusercontent.com";
const VALUE = "not-a-real-value-0123456789";
const SECTION_NAMES = [
  "board",
  "sessions",
  "census",
  "asks",
  "questionCount",
  "questions",
  "prs",
  "priorities",
  "agents",
  "services",
  "more",
];

function setup(answer: (argv: string[], stdin?: string) => Partial<RunResult>) {
  const calls: { argv: string[]; stdin?: string }[] = [];
  const ops = new Ops(readConfig({ HOME: "/h" }), async (argv, opts) => {
    calls.push({ argv, stdin: opts?.stdin });
    return { code: 0, stdout: "", stderr: "", ...answer(argv, opts?.stdin) };
  });
  return { ops, calls, table: actions(ops, new Work(ops)) };
}

describe("sign-in", () => {
  test("which methods are on is read through the setup script, with what Google must be told", async () => {
    const w = setup(() => ({
      stdout: 'note: something\n{"google": false, "pin": true, "team": "team-1.cloudflareaccess.com"}\n',
    }));
    expect(await w.ops.signin()).toEqual({
      google: false,
      pin: true,
      origin: "https://team-1.cloudflareaccess.com",
      redirect: "https://team-1.cloudflareaccess.com/cdn-cgi/access/callback",
    });
    expect(w.calls).toHaveLength(1);
    expect(w.calls[0].argv[0]).toBe("python3");
    expect(w.calls[0].argv[1]).toMatch(/\/pm-stack\/console\/scripts\/expose\.py$/);
    expect(w.calls[0].argv.slice(2)).toEqual(["--status"]);
    // A script that failed, or said something that is not a team's domain, is no answer.
    for (const bad of [
      { code: 1, stdout: '{"google": true, "pin": true, "team": "t.cloudflareaccess.com"}' },
      { stdout: "not json" },
      { stdout: '{"google": true, "pin": true, "team": "evil.example/x?"}' },
      { stdout: '{"google": true, "pin": true}' },
    ]) {
      const failed = await setup(() => bad)
        .ops.signin()
        .catch((error) => error);
      expect(failed).toBeInstanceOf(OpError);
      expect(failed.status).toBe(502);
    }
  });

  test("a Google client goes to the script's standard input and nowhere else", async () => {
    const w = setup(() => ({
      stdout: "changed: access: add Google sign-in\nchanged: access: application for pm.example\n",
    }));
    const docs = new Docs(
      Object.fromEntries(SECTION_NAMES.map((name) => [name, { ttlMs: 1000, load: async () => ({}) }])),
    );
    const result = await perform(w.table, docs, "sso.google", { clientId: ` ${CLIENT} `, clientSecret: `${VALUE}\n` });
    expect(result).toEqual({
      google: true,
      changed: ["access: add Google sign-in", "access: application for pm.example"],
    });
    expect(w.calls).toHaveLength(1);
    expect(w.calls[0].argv.slice(2)).toEqual(["--google-stdin"]);
    expect(w.calls[0].stdin).toBe(`${CLIENT}\n${VALUE}\n`);
    // Not on the command line, where any process listing would show it.
    expect(w.calls[0].argv.join(" ")).not.toContain(VALUE);
    expect(w.calls[0].argv.join(" ")).not.toContain(CLIENT);
    // Not in the record of what was done, and never kept to be done late.
    expect(w.table["sso.google"].audit({ clientId: CLIENT, clientSecret: VALUE })).toEqual({});
    expect(HOLDABLE.has("sso.google")).toBe(false);
    expect(JSON.stringify(result)).not.toContain(VALUE);
  });

  test("what is not a Google client is refused before the script is run", async () => {
    const w = setup(() => ({}));
    for (const args of [
      { clientId: "", clientSecret: VALUE },
      { clientId: "my-client", clientSecret: VALUE },
      { clientId: `${CLIENT}.evil.example`, clientSecret: VALUE },
      { clientId: CLIENT, clientSecret: "" },
      { clientId: CLIENT, clientSecret: "short" },
      { clientId: CLIENT, clientSecret: "two words here" },
      { clientId: CLIENT, clientSecret: `${VALUE}\n--status` },
      { clientId: CLIENT },
    ]) {
      const refused = await w.ops
        .googleSignin((args as { clientId: string }).clientId, (args as { clientSecret?: string }).clientSecret)
        .catch((error) => error);
      expect(refused).toBeInstanceOf(OpError);
      expect(refused.status).toBe(400);
      expect(refused.message).not.toContain(VALUE);
    }
    expect(w.calls).toEqual([]);
  });

  test("the owner's switch for the edge acting by itself runs the setup script here, on or off", async () => {
    const w = setup((argv) => ({
      stdout: argv.includes("on")
        ? "changed: edge direct: upload the GitHub and queue tokens and turn direct actions on\n"
        : "changed: edge direct: turn direct actions off and take the two tokens away\n",
    }));
    const docs = new Docs(
      Object.fromEntries(SECTION_NAMES.map((name) => [name, { ttlMs: 1000, load: async () => ({}) }])),
    );
    expect(await perform(w.table, docs, "edge.direct", { on: true })).toEqual({
      on: true,
      changed: ["edge direct: upload the GitHub and queue tokens and turn direct actions on"],
    });
    expect(await perform(w.table, docs, "edge.direct", { on: false })).toMatchObject({ on: false });
    expect(w.calls.map((c) => c.argv.slice(2))).toEqual([
      ["--edge-direct", "on"],
      ["--edge-direct", "off"],
    ]);
    // Nothing is handed to it: the tokens are read by the script, by name.
    expect(w.calls.every((c) => c.stdin === undefined)).toBe(true);
    // What it records is the choice. It is the laptop's to do, so it is never held for later.
    expect(w.table["edge.direct"].audit({ on: true })).toEqual({ on: true });
    expect(HOLDABLE.has("edge.direct")).toBe(false);
    // Anything but true or false is refused before the script is run.
    for (const on of ["on", 1, undefined, null]) {
      const refused = await w.ops.edgeDirect(on).catch((error) => error);
      expect(refused).toBeInstanceOf(OpError);
      expect(refused.status).toBe(400);
    }
    expect(w.calls).toHaveLength(2);
    // A script that fails says why.
    const failing = setup(() => ({ code: 1, stderr: "secret turso.fleet-queue-rw-token is not readable here\n" }));
    const failed = await failing.ops.edgeDirect(true).catch((error) => error);
    expect(failed.status).toBe(502);
    expect(failed.message).toBe(
      "Cloudflare was not given the tokens: secret turso.fleet-queue-rw-token is not readable here",
    );
  });

  test("a script that fails says why, without either value even if the script printed one", async () => {
    const w = setup(() => ({
      code: 1,
      stdout: "",
      stderr: `cloudflare POST /accounts/a/access/identity_providers: [{"message": "invalid client"}]\nechoed ${VALUE} by mistake\n`,
    }));
    const failed = await w.ops.googleSignin(CLIENT, VALUE).catch((error) => error);
    expect(failed).toBeInstanceOf(OpError);
    expect(failed.status).toBe(502);
    expect(failed.message).toBe(
      'Google sign-in was not turned on: cloudflare POST /accounts/a/access/identity_providers: [{"message": "invalid client"}]',
    );
    expect(failed.message).not.toContain(VALUE);
  });
});

test("the Android build on the laptop is described for the edge, and fingerprinted only when it changes", async () => {
  const { androidBuild } = await import("../src/server.ts");
  const { mkdtempSync, writeFileSync, utimesSync, rmSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const dir = mkdtempSync(join(tmpdir(), "pm-apk-"));
  try {
    const path = join(dir, "pm-console.apk");
    const build = androidBuild(path);
    // No build installed: nothing to describe.
    expect(await build()).toBeNull();
    writeFileSync(path, "first build");
    const one = await build();
    // SHA-256 of the text, worked out separately with sha256sum.
    expect(one?.sha256).toBe("f3adf3a4014af2dc15376ed0b22ac63cae9073430512bad33659b8f352c9fb30");
    expect(one?.size).toBe(11);
    expect(new TextDecoder().decode(await one?.read())).toBe("first build");
    // Unchanged: the same description.
    expect((await build())?.sha256).toBe(one?.sha256);
    // A new build put in its place, same length: a new fingerprint and date.
    writeFileSync(path, "other build");
    utimesSync(path, new Date(Date.now() + 5_000), new Date(Date.now() + 5_000));
    const two = await build();
    expect(two?.sha256).not.toBe(one?.sha256);
    expect(two?.at).toBeGreaterThan(one?.at ?? 0);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
