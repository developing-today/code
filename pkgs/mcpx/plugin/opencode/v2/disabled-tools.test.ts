/**
 * plugin.disabledTools, observed through the real v2 setup (#8472).
 *
 * The disabledTools setting — MCPX_PLUGIN_DISABLED_TOOLS, a comma-
 * or space-separated list of model-facing mcpx tool names — hides
 * the named tools from every session this plugin serves (#5122):
 * setup records the resolved list as each new session's tool-filter
 * policy in ctx.storage on the session's first sighting, and the
 * session "context" hook deletes the named tools from the
 * per-request tools record core builds, from the session's next
 * turn on (v2/tool-filter.ts). resolveOptions reads it host option
 * first, then the environment, then unset (nothing hidden).
 * tool-filter.test.ts covers the filter itself and the host-option
 * path through setup; this suite covers the environment path — the
 * way a user who sets the variable in the shell reaches the same
 * effect. The setting lives entirely inside this plugin host, so
 * this suite is where its effect can be observed: setup is the real
 * setupMcpxV2 with the stub context shape tool-filter.test.ts uses
 * (Map-backed storage, a captured context hook, a no-op tool
 * domain), and the cases fire the hook with the plain tools record
 * the host would build and read what the model would see. The Go
 * effect probe for the setting
 * (TestPluginDisabledToolsHidesToolsFromTheEnvironment in
 * internal/e2e) runs this file. The cases set the setting the way a
 * user would — MCPX_PLUGIN_DISABLED_TOOLS in the process
 * environment, or the host option — and assert the observable
 * with/without differences the effect guard (#1971) demands:
 * unset hides nothing and seeds no policy; a named tool is hidden
 * from the second turn while its siblings and the host tools stay;
 * a separated list hides each named tool; a name outside the mcpx_
 * namespace can never hide a host tool; and the host option beats
 * the environment in both directions.
 *
 *   bun test plugin/opencode/v2/disabled-tools.test.ts
 */
import { afterEach, describe, expect, test } from "bun:test";
import { setupMcpxV2, type McpxV2Context } from "./index.ts";
import type { Options } from "./options.ts";
import type { StorageLike } from "./remember.ts";
import { readDisabledTools, readDisabledToolsPolicy } from "./tool-filter.ts";

const ENV_KEY = "MCPX_PLUGIN_DISABLED_TOOLS";
const savedEnv = process.env[ENV_KEY];
afterEach(() => {
  if (savedEnv === undefined) delete process.env[ENV_KEY];
  else process.env[ENV_KEY] = savedEnv;
});

const stubStorage = (): StorageLike & { map: Map<string, unknown> } => {
  const map = new Map<string, unknown>();
  return {
    map,
    get: async (k) => map.get(k),
    set: async (k, v) => void map.set(k, v),
    remove: async (k) => void map.delete(k),
  };
};

const toolDef = (description: string) => ({ description, input: {} });

// The tools record core builds for one model request: two mcpx
// tools and two host tools (the tool-filter.test.ts shape).
const requestTools = () => ({
  mcpx_exec: toolDef("exec"),
  mcpx_daemon_status: toolDef("status"),
  read: toolDef("host read"),
  bash: toolDef("host bash"),
});

type ContextInput = {
  sessionID: string;
  system: unknown[];
  tools: ReturnType<typeof requestTools>;
};

// Setup exactly as the host performs it (v2/index.ts): the stub
// context captures the registered "context" hook so the test can
// fire it with the plain input objects the host would build.
const stubCtx = (storage: StorageLike) => {
  let contextFn: ((input: never) => void) | undefined;
  const ctx: McpxV2Context = {
    storage,
    directory: "/work",
    session: {
      hook: (_name, fn) => {
        contextFn = fn as (input: never) => void;
      },
    },
    tool: {
      hook: () => {},
      transform: async () => ({}),
    },
  };
  const fire = (sessionID: string): ContextInput => {
    const input: ContextInput = { sessionID, system: [], tools: requestTools() };
    contextFn!(input as never);
    return input;
  };
  return { ctx, fire };
};

// The seeding write and the snapshot refresh are async (the hook
// never waits on storage): let them land before the next turn.
const flush = async (): Promise<void> => {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
};

const setupWith = async (env: string | undefined, rawOptions?: Options) => {
  if (env === undefined) delete process.env[ENV_KEY];
  else process.env[ENV_KEY] = env;
  const storage = stubStorage();
  const { ctx, fire } = stubCtx(storage);
  // tools/daemonTools off, as in tool-filter.test.ts: registration
  // is not what this suite observes, and neither option touches the
  // disabledTools resolution (host option, then env, then unset).
  await setupMcpxV2(ctx, { tools: false, daemonTools: false, ...rawOptions });
  return { storage, fire };
};

const names = (input: ContextInput): string[] => Object.keys(input.tools).sort();
const ALL_TOOLS = ["bash", "mcpx_daemon_status", "mcpx_exec", "read"];

describe("plugin.disabledTools (MCPX_PLUGIN_DISABLED_TOOLS)", () => {
  test("unset: nothing is hidden on any turn and no policy is seeded", async () => {
    const { storage, fire } = await setupWith(undefined);
    expect(names(fire("s1"))).toEqual(ALL_TOOLS);
    await flush();
    expect(await readDisabledToolsPolicy(storage, "s1")).toBeUndefined();
    expect(names(fire("s1"))).toEqual(ALL_TOOLS);
  });

  test("a named tool is hidden from the second turn; siblings and host tools stay", async () => {
    const { storage, fire } = await setupWith("mcpx_exec");
    // First turn: no snapshot exists yet, so nothing is hidden — the
    // seeding write the hook just started is what the refresh loads.
    expect(fire("s1").tools.mcpx_exec).toBeDefined();
    await flush();
    expect([...(await readDisabledTools(storage, "s1"))]).toEqual(["mcpx_exec"]);
    const second = fire("s1");
    expect(second.tools.mcpx_exec).toBeUndefined();
    expect(second.tools.mcpx_daemon_status).toBeDefined();
    expect(second.tools.read).toBeDefined();
    expect(second.tools.bash).toBeDefined();
  });

  test("a separated list hides each named tool", async () => {
    const { fire } = await setupWith("mcpx_exec, mcpx_daemon_status");
    fire("s1");
    await flush();
    expect(names(fire("s1"))).toEqual(["bash", "read"]);
  });

  test("a name outside the mcpx_ namespace can never hide a host tool", async () => {
    const { fire } = await setupWith("mcpx_exec, read");
    fire("s1");
    await flush();
    const second = fire("s1");
    expect(second.tools.mcpx_exec).toBeUndefined();
    expect(second.tools.read).toBeDefined();
  });

  test("the host option beats the environment in both directions", async () => {
    const opt = await setupWith("mcpx_exec", { disabledTools: ["mcpx_daemon_status"] });
    opt.fire("s1");
    await flush();
    const optSecond = opt.fire("s1");
    expect(optSecond.tools.mcpx_daemon_status).toBeUndefined();
    expect(optSecond.tools.mcpx_exec).toBeDefined();

    const off = await setupWith("mcpx_exec", { disabledTools: [] });
    off.fire("s1");
    await flush();
    expect(off.fire("s1").tools.mcpx_exec).toBeDefined();
  });
});
