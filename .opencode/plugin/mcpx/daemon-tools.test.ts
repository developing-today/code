import { afterEach, describe, expect, mock, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// The daemonTools setting — the plugin option `daemonTools`, or
// MCPX_PLUGIN_DAEMON_TOOLS in the environment — decides whether the
// session plugin offers the three daemon tools (mcpx_daemon_status,
// mcpx_daemon_select, mcpx_daemon_forget). The resolution is a
// tri-state, in the plugin's usual order:
//
//   plugin option  ??  truthy(env)  ??  (plugin.tools || boot.ambiguous)
//
// "auto" — the registry default — is not a decision on either of the
// first two rungs: as an environment value it parses to undefined,
// so the fallback decides. The fallback's ambiguity arm needs a real
// multi-daemon discovery, so these cases isolate it away (empty
// XDG state/runtime dirs: the boot scan finds no daemon info files
// and boot.ambiguous is false) and observe the fallback through the
// plugin.tools switch instead. What is asserted is the returned
// hooks' `tool` map — the exact surface opencode offers the model.

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
  // env.test.ts pattern): replacing process.env wholesale does not
  // restore the native environment other code reads.
  for (const key of Object.keys(process.env)) {
    if (!(key in savedEnv)) delete process.env[key];
  }
  Object.assign(process.env, savedEnv);
  for (const dir of scratch.splice(0)) rmSync(dir, { recursive: true, force: true });
});

const DAEMON_TOOLS = ["mcpx_daemon_status", "mcpx_daemon_select", "mcpx_daemon_forget"];

// loadTools loads the real session plugin with daemon discovery
// isolated to empty state/runtime directories, applies the given
// environment overrides (an undefined value deletes the variable),
// passes the given plugin options, and returns the hooks' tool map.
const loadTools = async (opts: {
  env?: Record<string, string | undefined>;
  options?: Record<string, unknown>;
}): Promise<Record<string, unknown> | undefined> => {
  const state = mkdtempSync(join(REAL_TMP, "mcpxt-dt-state-"));
  const runtime = mkdtempSync(join(REAL_TMP, "mcpxt-dt-run-"));
  const dir = mkdtempSync(join(REAL_TMP, "mcpxt-dt-"));
  scratch.push(state, runtime, dir);
  process.env.XDG_STATE_HOME = state;
  process.env.XDG_RUNTIME_DIR = runtime;
  // Nothing ambient may reach a rung of the resolution: no named
  // endpoint or socket, and the two switches start deleted.
  delete process.env.MCPX_DAEMON_ENDPOINT;
  delete process.env.MCPX_ENDPOINT;
  delete process.env.MCPX_SOCKET;
  delete process.env.MCPX_PLUGIN_DAEMON_TOOLS;
  delete process.env.MCPX_PLUGIN_TOOLS;
  for (const [key, value] of Object.entries(opts.env ?? {})) {
    if (value === undefined) delete process.env[key];
    else process.env[key] = value;
  }
  const client = {
    session: {
      get: async () => ({
        data: { id: "ses_1", title: "t", directory: dir, version: "1", time: { created: Date.now() } },
      }),
    },
    tui: { showToast: async () => {} },
  };
  const hooks: any = await (plugin as any)(
    { directory: dir, worktree: dir, project: { id: "p1" }, client },
    { headless: true, ...(opts.options ?? {}) },
  );
  return hooks.tool as Record<string, unknown> | undefined;
};

describe("plugin.daemonTools gating of the daemon tools", () => {
  test("environment true offers the daemon tools while the main tools stay off", async () => {
    const tools = await loadTools({ env: { MCPX_PLUGIN_DAEMON_TOOLS: "true" } });
    for (const name of DAEMON_TOOLS) expect(tools?.[name]).toBeDefined();
    expect(tools?.mcpx_discover).toBeUndefined();
  });

  test("environment false withholds them even when the main tools are on", async () => {
    const tools = await loadTools({
      env: { MCPX_PLUGIN_DAEMON_TOOLS: "false", MCPX_PLUGIN_TOOLS: "true" },
    });
    expect(tools?.mcpx_discover).toBeDefined();
    for (const name of DAEMON_TOOLS) expect(tools?.[name]).toBeUndefined();
  });

  test("the plugin option beats the environment: false over true", async () => {
    const tools = await loadTools({
      env: { MCPX_PLUGIN_DAEMON_TOOLS: "true" },
      options: { daemonTools: false },
    });
    // Main tools are off too, so no tool map is returned at all.
    expect(tools).toBeUndefined();
  });

  test("the plugin option beats the environment: true over false", async () => {
    const tools = await loadTools({
      env: { MCPX_PLUGIN_DAEMON_TOOLS: "false" },
      options: { daemonTools: true },
    });
    for (const name of DAEMON_TOOLS) expect(tools?.[name]).toBeDefined();
    expect(tools?.mcpx_discover).toBeUndefined();
  });

  test("unset everywhere, the fallback follows the main tools: on offers them", async () => {
    const tools = await loadTools({ env: { MCPX_PLUGIN_TOOLS: "true" } });
    expect(tools?.mcpx_discover).toBeDefined();
    for (const name of DAEMON_TOOLS) expect(tools?.[name]).toBeDefined();
  });

  test("unset everywhere with the main tools off offers nothing", async () => {
    const tools = await loadTools({});
    expect(tools).toBeUndefined();
  });

  test("environment auto is not a decision: the fallback decides", async () => {
    const off = await loadTools({ env: { MCPX_PLUGIN_DAEMON_TOOLS: "auto" } });
    expect(off).toBeUndefined();
    const on = await loadTools({
      env: { MCPX_PLUGIN_DAEMON_TOOLS: "auto", MCPX_PLUGIN_TOOLS: "true" },
    });
    for (const name of DAEMON_TOOLS) expect(on?.[name]).toBeDefined();
  });
});
