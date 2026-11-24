/**
 * plugin.env, observed through the real shell.env hook (#8472).
 *
 * The env setting — MCPX_PLUGIN_ENV, one of minimal / standard /
 * full — decides how much the plugin host tells a shell command
 * about the session it runs in. The level is read once when the
 * plugin loads (mcpx-session.ts) and gates the "shell.env" hook:
 * minimal injects only MCPX_SESSION_ID; standard adds the call,
 * project and harness block; full additionally describes the
 * session (an opencode client round-trip) and injects the session
 * block. The choice lives entirely inside this plugin host, so
 * this suite is where its effect can be observed. The Go effect
 * probe for the setting
 * (TestPluginEnvLevelChangesShellEnvFromTheEnvironment in
 * internal/e2e) runs this file. The cases set the setting the way
 * a user would — MCPX_PLUGIN_ENV in the process environment
 * before the plugin loads — drive the ordinary caller (the
 * shell.env hook opencode invokes for every shell command), and
 * assert the observable with/without differences the effect guard
 * (#1971) demands, including that the session is only ever
 * described at full.
 *
 *   bun test plugin/opencode/mcpx/env.test.ts
 */
import { afterEach, describe, expect, mock, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// The plugin package has no installed node_modules in the repo
// checkout; only the `tool` helper is needed to build the hooks.
const schemaStub: any = new Proxy(function () {}, {
  get: () => schemaStub,
  apply: () => schemaStub,
});
mock.module("@opencode-ai/plugin", () => ({
  tool: Object.assign((x: unknown) => x, { schema: schemaStub }),
}));
// @ts-ignore: the session plugin exports only its default (loader glob).
const plugin = (await import("../mcpx-session.ts")).default;

// Captured before any test runs: tmpdir() reads TMPDIR, and the
// environment restore below must not strand it.
const REAL_TMP = tmpdir();
const scratch: string[] = [];
const savedEnv = { ...process.env };
afterEach(() => {
  // Restore in place, on the original process.env object (the
  // instructions.test.ts pattern): replacing process.env wholesale
  // does not restore the native environment other code reads.
  for (const key of Object.keys(process.env)) {
    if (!(key in savedEnv)) delete process.env[key];
  }
  Object.assign(process.env, savedEnv);
  for (const dir of scratch.splice(0)) rmSync(dir, { recursive: true, force: true });
});

const CREATED = Date.now() - 60_000;

// shellEnv loads the plugin with MCPX_PLUGIN_ENV set to `level`
// (deleted from the environment when undefined) and runs one
// shell.env hook invocation against a stub opencode client whose
// session.get calls are counted, so a level that describes the
// session is observable as a round-trip as well as by its vars.
const shellEnv = async (level: string | undefined) => {
  if (level === undefined) delete process.env.MCPX_PLUGIN_ENV;
  else process.env.MCPX_PLUGIN_ENV = level;
  const dir = mkdtempSync(join(REAL_TMP, "mcpxt-env-"));
  scratch.push(dir);
  let gets = 0;
  const client = {
    session: {
      get: async () => {
        gets += 1;
        return {
          data: {
            id: "ses_1",
            title: "a title",
            directory: dir,
            version: "1.2.3",
            time: { created: CREATED },
          },
        };
      },
    },
    tui: { showToast: async () => {} },
  };
  const hooks: any = await (plugin as any)(
    { directory: dir, worktree: dir, project: { id: "p1" }, client },
    { headless: true },
  );
  const output: any = { env: {} };
  await hooks["shell.env"]({ cwd: join(dir, "sub"), sessionID: "ses_1", callID: "call_1" }, output);
  return { env: output.env as Record<string, string>, gets, dir };
};

describe("plugin.env levels in the shell.env hook", () => {
  test("minimal injects the session id and nothing else", async () => {
    const { env, gets } = await shellEnv("minimal");
    expect(env.MCPX_SESSION_ID).toBe("ses_1");
    expect(env.MCPX_OPENCODE_CWD).toBeUndefined();
    expect(env.MCPX_CALL_ID).toBeUndefined();
    expect(env.MCPX_HARNESS).toBeUndefined();
    expect(env.MCPX_SESSION_TITLE).toBeUndefined();
    expect(gets).toBe(0);
  });

  test("standard adds the call and harness block but never describes the session", async () => {
    const { env, gets, dir } = await shellEnv("standard");
    expect(env.MCPX_SESSION_ID).toBe("ses_1");
    expect(env.MCPX_OPENCODE_CWD).toBe(join(dir, "sub"));
    expect(env.MCPX_CALL_ID).toBe("call_1");
    expect(env.MCPX_OPENCODE_DIRECTORY).toBe(dir);
    expect(env.MCPX_PROJECT_ID).toBe("p1");
    expect(env.MCPX_HARNESS).toBe("opencode");
    expect(env.MCPX_SHELL_SEQ).toBe("1");
    expect(env.MCPX_SESSION_TITLE).toBeUndefined();
    expect(env.MCPX_SESSION_DIRECTORY).toBeUndefined();
    expect(env.MCPX_SESSION_CREATED).toBeUndefined();
    expect(gets).toBe(0);
  });

  test("full additionally injects the described session block", async () => {
    const { env, gets, dir } = await shellEnv("full");
    expect(env.MCPX_OPENCODE_CWD).toBe(join(dir, "sub"));
    expect(env.MCPX_SESSION_TITLE).toBe("a title");
    expect(env.MCPX_SESSION_DIRECTORY).toBe(dir);
    expect(env.MCPX_SESSION_VERSION).toBe("1.2.3");
    expect(env.MCPX_SESSION_DEPTH).toBe("0");
    expect(env.MCPX_SESSION_CREATED).toBe(new Date(CREATED).toISOString());
    expect(Number(env.MCPX_SESSION_AGE_MS)).toBeGreaterThanOrEqual(60_000);
    expect(gets).toBeGreaterThanOrEqual(1);
  });

  test("unset defaults to full", async () => {
    const { env, gets } = await shellEnv(undefined);
    expect(env.MCPX_SESSION_TITLE).toBe("a title");
    expect(gets).toBeGreaterThanOrEqual(1);
  });

  test("an unrecognized level is treated as full", async () => {
    const { env, gets } = await shellEnv("verbose");
    expect(env.MCPX_SESSION_TITLE).toBe("a title");
    expect(gets).toBeGreaterThanOrEqual(1);
  });
});
