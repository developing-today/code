/**
 * plugin.backend, observed through the real tool surface (#8472).
 *
 * The backend setting — MCPX_PLUGIN_BACKEND, one of auto / v1 / cli —
 * decides how the plugin host reaches mcpx on every tool call: `auto`
 * prefers the daemon socket and falls back to spawning the binary,
 * `v1` uses the socket or refuses, and `cli` never uses the socket.
 * The choice lives entirely inside this plugin host (tools.ts's
 * viaDaemon / viaCLI), so this suite is where its effect can be
 * observed. The Go effect probe for the setting
 * (TestPluginBackendChoosesDaemonOrBinaryFromTheEnvironment in
 * internal/e2e) runs this file. The cases set the setting the way a
 * user would — MCPX_PLUGIN_BACKEND through resolveOptions — drive
 * the ordinary caller (the discover tool executor) against a stub
 * discovery, and assert the observable with/without difference the
 * effect guard (#1971) demands: who answered (daemon namespaces vs
 * the binary's output vs the v1 refusal), and who was never asked.
 *
 *   bun test plugin/opencode/v2/backend.test.ts
 */
import { describe, expect, test } from "bun:test";
import type { Candidate, Discovery, Target } from "../../core/daemon.ts";
import { resolveOptions } from "./options.ts";
import type { StorageLike } from "./remember.ts";
import { createToolRuntime, type DaemonClientLike } from "./tools.ts";

const socketTarget: Target = { kind: "socket", path: "/tmp/m.sock" };

const candidate = (target: Target): Candidate => ({
  target,
  source: "info-file",
  configPath: "/cfg/mcpx.json",
});

const discoveryWith = (candidates: Candidate[]): Discovery => ({
  target: candidates[0]?.target,
  chosen: candidates[0],
  candidates,
  rung: 2,
  reason: "info-file",
  ambiguous: candidates.length > 1,
});

const stubStorage = (): StorageLike => {
  const map = new Map<string, unknown>();
  return {
    get: async (k) => map.get(k),
    set: async (k, v) => void map.set(k, v),
    remove: async (k) => void map.delete(k),
  };
};

const discoverWithEnv = async (env: Record<string, string | undefined>, candidates: Candidate[]) => {
  const calls = { namespaces: 0, bin: 0 };
  const fakeClient = (): DaemonClientLike => ({
    namespaces: async () => {
      calls.namespaces += 1;
      return [{ namespace: "files", tools: 3, description: "fs" }];
    },
    types: async () => "files.read(path: string)",
    exec: async () => ({ output: "ran it" }),
    logQuery: async () => ({ records: [{ id: 1 }] }),
    stats: async () => ({ rows: [] }),
    putSetting: async () => "ok",
    shutdown: async () => true,
    record: async () => ({}),
  });
  const rt = createToolRuntime({
    directory: "/work",
    env: {},
    options: resolveOptions(undefined, env),
    storage: stubStorage(),
    discoverFn: async () => discoveryWith(candidates),
    runBinFn: async () => {
      calls.bin += 1;
      return "from the binary";
    },
    clientFor: () => fakeClient(),
    assertPermission: async () => {},
  } as never);
  const discover = rt.tools().find((t) => t.name === "discover")!;
  const out = await discover.execute({} as never, { sessionID: "s1" });
  return { content: out.content, calls };
};

describe("plugin.backend (MCPX_PLUGIN_BACKEND)", () => {
  test("auto (the default) answers from the daemon when one is reachable, the binary untouched", async () => {
    const { content, calls } = await discoverWithEnv({}, [candidate(socketTarget)]);
    expect(content).toContain("files  (3 tools)");
    expect(calls.namespaces).toBe(1);
    expect(calls.bin).toBe(0);
  });

  test("cli never uses the socket: the binary answers even with a daemon reachable", async () => {
    const { content, calls } = await discoverWithEnv({ MCPX_PLUGIN_BACKEND: "cli" }, [candidate(socketTarget)]);
    expect(content).toContain("from the binary");
    expect(content).not.toContain("files  (3 tools)");
    expect(calls.namespaces).toBe(0);
    expect(calls.bin).toBe(1);
  });

  test("v1 with no daemon reachable refuses instead of running the binary", async () => {
    const { content, calls } = await discoverWithEnv({ MCPX_PLUGIN_BACKEND: "v1" }, []);
    expect(content).toContain("backend=v1 forbids running the binary");
    expect(calls.namespaces).toBe(0);
    expect(calls.bin).toBe(0);
  });

  test("auto with no daemon reachable falls back to the binary — the fallback v1 refuses", async () => {
    const { content, calls } = await discoverWithEnv({}, []);
    expect(content).toContain("from the binary");
    expect(calls.bin).toBe(1);
  });

  test("v1 still answers from the daemon when one is reachable", async () => {
    const { content, calls } = await discoverWithEnv({ MCPX_PLUGIN_BACKEND: "v1" }, [candidate(socketTarget)]);
    expect(content).toContain("files  (3 tools)");
    expect(calls.namespaces).toBe(1);
    expect(calls.bin).toBe(0);
  });
});
