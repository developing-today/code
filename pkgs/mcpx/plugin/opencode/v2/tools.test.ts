/**
 * The v2 tool surface, against stub contexts: a recording ToolEditor, a
 * Map-backed storage, and injected discover/runBin/client fakes. The v2
 * host is not runnable here, so the stubs are the contract — each one is
 * shaped after the verified facts in docs/opencode-plugin-v2.md (the
 * namespace question in §7 is resolved there: editor.namespace takes
 * { name, description }, and the model-facing name of a tool added as
 * `exec` under namespace `mcpx` is `mcpx_exec`).
 *
 *   bun test plugin/opencode/v2/tools.test.ts
 */
import { describe, expect, test } from "bun:test";
import { rank, targetKey, type Candidate, type Discovery, type DiscoverOptions, type Target } from "../mcpx/daemon.ts";
import { resolveOptions } from "./options.ts";
import { setupMcpxV2, type McpxV2Context } from "./index.ts";
import { rememberDaemon, type StorageLike } from "./remember.ts";
import {
  createToolRuntime,
  registerTools,
  type DaemonClientLike,
  type McpxToolInfo,
  type ToolDomainLike,
  type ToolEditorLike,
} from "./tools.ts";

const stubStorage = (): StorageLike & { map: Map<string, unknown> } => {
  const map = new Map<string, unknown>();
  return {
    map,
    get: async (k) => map.get(k),
    set: async (k, v) => void map.set(k, v),
    remove: async (k) => void map.delete(k),
  };
};

const stubEditor = (): ToolEditorLike & {
  namespaces: { name: string; description: string }[];
  added: McpxToolInfo[];
} => {
  const namespaces: { name: string; description: string }[] = [];
  const added: McpxToolInfo[] = [];
  return {
    namespaces,
    added,
    namespace: (ns) => void namespaces.push(ns),
    add: (t) => void added.push(t),
  };
};

const stubDomain = (
  editor: ReturnType<typeof stubEditor>,
): ToolDomainLike => ({
  transform: async (fn) => {
    fn(editor);
    return {};
  },
});

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

const fakeClient = (over: Partial<DaemonClientLike> = {}): DaemonClientLike => ({
  namespaces: async () => [{ namespace: "files", tools: 3, description: "fs" }],
  types: async () => "files.read(path: string)",
  exec: async () => ({ output: "ran it" }),
  logQuery: async () => ({ records: [{ id: 1 }] }),
  stats: async () => ({ rows: [] }),
  putSetting: async () => "ok",
  shutdown: async () => true,
  record: async () => ({}),
  ...over,
});

const runtime = (over: Record<string, unknown> = {}) =>
  createToolRuntime({
    directory: "/work",
    env: {},
    options: { backend: "auto", remember: "session", annotate: true },
    storage: stubStorage(),
    discoverFn: async () => discoveryWith([candidate(socketTarget)]),
    runBinFn: async () => "from the binary",
    clientFor: () => fakeClient(),
    ...over,
  } as never);

describe("registration (tool.transform)", () => {
  test("namespace plus the full tool set yields the v1 model-facing names", async () => {
    const editor = stubEditor();
    const names = await registerTools(stubDomain(editor), runtime(), {
      toolsEnabled: true,
      daemonTools: true,
    });
    expect(editor.namespaces).toEqual([
      { name: "mcpx", description: expect.stringContaining("mcpx") },
    ]);
    expect(names.sort()).toEqual([
      "mcpx_daemon_forget",
      "mcpx_daemon_select",
      "mcpx_daemon_status",
      "mcpx_discover",
      "mcpx_exec",
      "mcpx_observe",
    ]);
    for (const t of editor.added) expect(t.options?.namespace).toBe("mcpx");
  });

  test("tools off and daemon tools off adds no tools", async () => {
    const editor = stubEditor();
    const rt = runtime({ options: { backend: "auto", tools: false, daemonTools: false } });
    const names = await registerTools(stubDomain(editor), rt, {
      toolsEnabled: false,
      daemonTools: false,
    });
    expect(names).toEqual([]);
    expect(editor.added).toEqual([]);
  });

  test("daemon tools default on when boot discovery is ambiguous", async () => {
    const editor = stubEditor();
    const two = discoveryWith([candidate(socketTarget), candidate({ kind: "url", base: "http://127.0.0.1:9" })]);
    const rt = runtime({ discoverFn: async () => two });
    const names = await registerTools(stubDomain(editor), rt, {
      toolsEnabled: false,
      daemonTools: undefined,
    });
    expect(names).toContain("mcpx_daemon_status");
    expect(names).not.toContain("mcpx_exec");
  });
});

describe("setup: instructions gate (plugin.instructions)", () => {
  const stubCtx = (): {
    ctx: McpxV2Context;
    fireContext: (sessionID: string) => { system: unknown[] };
  } => {
    let contextFn: ((input: never) => void) | undefined;
    const ctx: McpxV2Context = {
      storage: stubStorage(),
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
    return {
      ctx,
      fireContext: (sessionID) => {
        const input = { sessionID, system: [] as unknown[] };
        contextFn!(input as never);
        return input;
      },
    };
  };

  test("paragraph stays out when instructions is off (the default)", async () => {
    const { ctx, fireContext } = stubCtx();
    await setupMcpxV2(ctx, { tools: false, daemonTools: false });
    expect(fireContext("s1").system.length).toBe(0);
  });

  test("paragraph is appended when instructions is on", async () => {
    const { ctx, fireContext } = stubCtx();
    await setupMcpxV2(ctx, { tools: false, daemonTools: false, instructions: true });
    expect(fireContext("s1").system.length).toBe(1);
  });
});

describe("options resolution (v1 parity)", () => {
  test("an explicit empty bin is preserved — it forbids spawning", () => {
    expect(resolveOptions({ bin: "" }, {}).bin).toBe("");
    expect(
      resolveOptions({ bin: "" }, { MCPX_PLUGIN_BIN: "/usr/bin/mcpx" }).bin,
    ).toBe("");
  });

  test("bin falls back to the environment, then to unset", () => {
    expect(resolveOptions({}, { MCPX_PLUGIN_BIN: "/opt/mcpx" }).bin).toBe("/opt/mcpx");
    expect(resolveOptions({}, {}).bin).toBeUndefined();
  });
});

describe("discovery ladder wiring", () => {
  test("the ladder never reads the v1 remember file", async () => {
    const seen: DiscoverOptions[] = [];
    const rt = runtime({
      discoverFn: async (opts: DiscoverOptions) => {
        seen.push(opts);
        return discoveryWith([candidate(socketTarget)]);
      },
    });
    await rt.look();
    expect(seen.length).toBeGreaterThan(0);
    for (const opts of seen) expect(opts.useRememberFile).toBe(false);
  });

  test("onTarget fires once for the settled daemon, again on select", async () => {
    const targets: Target[] = [];
    const other: Target = { kind: "url", base: "http://127.0.0.1:9" };
    const rt = runtime({
      onTarget: (t: Target) => void targets.push(t),
      discoverFn: async () =>
        discoveryWith([candidate(socketTarget), candidate(other)]),
    });
    await rt.look();
    await rt.look();
    expect(targets).toEqual([socketTarget]);
    const select = rt.daemonTools().find((t) => t.name === "daemon_select")!;
    // daemon_select takes the index into the RANKED list, as status prints it.
    const idx = rank([candidate(socketTarget), candidate(other)], "/work").findIndex(
      (c) => targetKey(c.target) === targetKey(other),
    );
    await select.execute({ candidate: idx } as never, { sessionID: "s1" });
    expect(targets).toEqual([socketTarget, other]);
  });
});

describe("rediscovery precedence", () => {
  test("a session choice survives a transient failure — stored daemon does not reclaim it", async () => {
    const storage = stubStorage();
    const stored: Target = { kind: "socket", path: "/tmp/stored.sock" };
    const picked: Target = { kind: "socket", path: "/tmp/picked.sock" };
    await rememberDaemon(storage, "/work", stored);
    const rememberedSeen: (string | undefined)[] = [];
    let failExec = false;
    const rt = runtime({
      storage,
      options: {
        backend: "auto",
        remember: "session",
        annotate: true,
        discoveryRetryMs: 0,
      },
      discoverFn: async (opts: DiscoverOptions) => {
        rememberedSeen.push(opts.remembered ? targetKey(opts.remembered) : undefined);
        return discoveryWith([candidate(stored), candidate(picked)]);
      },
      clientFor: () =>
        fakeClient({
          exec: async () => {
            if (failExec) throw new Error("daemon went away");
            return { output: "ok" };
          },
        }),
    });
    // Boot: the stored daemon is what discovery settles on.
    await rt.look();
    // The user picks the other daemon for this session.
    const select = rt.daemonTools().find((t) => t.name === "daemon_select")!;
    const idx = rank([candidate(stored), candidate(picked)], "/work").findIndex(
      (c) => targetKey(c.target) === targetKey(picked),
    );
    await select.execute({ candidate: idx } as never, { sessionID: "s1" });
    // A transient failure clears the cached discovery (viaDaemon forgets).
    failExec = true;
    const exec = rt.tools().find((t) => t.name === "exec")!;
    await expect(
      exec.execute({ source: "1" } as never, { sessionID: "s1" }),
    ).rejects.toThrow();
    // Rediscovery must ask the ladder for the SESSION choice, not storage.
    rememberedSeen.length = 0;
    await rt.look();
    expect(rememberedSeen).toEqual([targetKey(picked)]);
  });
});

describe("tool executors", () => {
  test("daemon_status lists candidates and names the one in use", async () => {
    const rt = runtime();
    const status = rt.daemonTools().find((t) => t.name === "daemon_status")!;
    const out = await status.execute({} as never, { sessionID: "s1" });
    expect(out.content).toContain("1 daemon running");
    expect(out.content).toContain("unix:///tmp/m.sock");
  });

  test("discover answers from the daemon when one answers", async () => {
    const rt = runtime();
    const discoverTool = rt.tools().find((t) => t.name === "discover")!;
    const out = await discoverTool.execute({} as never, { sessionID: "s1" });
    expect(out.content).toContain("files  (3 tools)");
  });

  test("exec falls back to the binary when the daemon route is absent", async () => {
    const rt = runtime({
      clientFor: () => fakeClient({ exec: async () => undefined }),
    });
    const exec = rt.tools().find((t) => t.name === "exec")!;
    const out = await exec.execute({ source: "1+1" } as never, { sessionID: "s1" });
    expect(out.content).toContain("from the binary");
  });

  test("daemon_select with a bad index says what the range is", async () => {
    const rt = runtime();
    const select = rt.daemonTools().find((t) => t.name === "daemon_select")!;
    const out = await select.execute({ candidate: 7 } as never, { sessionID: "s1" });
    expect(out.content).toContain("lists 0..0");
  });

  test("daemon_select session scope holds the choice without writing storage", async () => {
    const storage = stubStorage();
    const rt = runtime({ storage });
    const select = rt.daemonTools().find((t) => t.name === "daemon_select")!;
    const out = await select.execute({ candidate: 0 } as never, { sessionID: "s1" });
    expect(out.content).toContain("Now using");
    expect(out.content).toContain("this session only");
    expect(storage.map.size).toBe(0);
  });

  test("daemon_select until-gone persists through ctx.storage", async () => {
    const storage = stubStorage();
    const rt = runtime({ storage });
    const select = rt.daemonTools().find((t) => t.name === "daemon_select")!;
    const out = await select.execute(
      { candidate: 0, remember: "until-gone" } as never,
      { sessionID: "s1" },
    );
    expect(out.content).toContain("until this daemon stops answering");
    expect(storage.map.get("daemon:/work")).toBeDefined();
  });
});
