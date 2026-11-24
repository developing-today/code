/**
 * plugin.annotate, observed through the real tool surface (#8472).
 *
 * The annotation — the "answered by" note mcpx appends to a tool
 * result when more than one daemon matches the directory — is produced
 * entirely inside this plugin host, so this suite is where its effect
 * can be observed. The Go effect probe for the setting
 * (TestPluginAnnotateChangesToolResultsFromTheEnvironment in
 * internal/e2e) runs this file. The cases set the setting the way a
 * user would — MCPX_PLUGIN_ANNOTATE through resolveOptions — drive
 * the ordinary caller (the discover tool executor) against an
 * ambiguous discovery, and assert the observable with/without
 * difference the effect guard (#1971) demands.
 *
 *   bun test plugin/opencode/v2/annotate.test.ts
 */
import { describe, expect, test } from "bun:test";
import type { Candidate, Discovery, Target } from "../../core/daemon.ts";
import { resolveOptions } from "./options.ts";
import type { StorageLike } from "./remember.ts";
import { createToolRuntime, type DaemonClientLike } from "./tools.ts";

const socketTarget: Target = { kind: "socket", path: "/tmp/m.sock" };
const urlTarget: Target = { kind: "url", base: "http://127.0.0.1:9" };

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

const fakeClient = (): DaemonClientLike => ({
  namespaces: async () => [{ namespace: "files", tools: 3, description: "fs" }],
  types: async () => "files.read(path: string)",
  exec: async () => ({ output: "ran it" }),
  logQuery: async () => ({ records: [{ id: 1 }] }),
  stats: async () => ({ rows: [] }),
  putSetting: async () => "ok",
  shutdown: async () => true,
  record: async () => ({}),
});

const discoverWithEnv = async (env: Record<string, string | undefined>, candidates: Candidate[]) => {
  const rt = createToolRuntime({
    directory: "/work",
    env: {},
    options: resolveOptions(undefined, env),
    storage: stubStorage(),
    discoverFn: async () => discoveryWith(candidates),
    runBinFn: async () => "from the binary",
    clientFor: () => fakeClient(),
    assertPermission: async () => {},
  } as never);
  const discover = rt.tools().find((t) => t.name === "discover")!;
  const out = await discover.execute({} as never, { sessionID: "s1" });
  return out.content;
};

describe("plugin.annotate (MCPX_PLUGIN_ANNOTATE)", () => {
  test("the note is appended when two daemons match and annotate is on (the default)", async () => {
    const content = await discoverWithEnv({}, [candidate(socketTarget), candidate(urlTarget)]);
    expect(content).toContain("files  (3 tools)");
    expect(content).toContain("mcpx: answered by");
    expect(content).toContain("2 daemons match this directory");
  });

  test("the note is gone when MCPX_PLUGIN_ANNOTATE=false, the answer unchanged", async () => {
    const content = await discoverWithEnv({ MCPX_PLUGIN_ANNOTATE: "false" }, [
      candidate(socketTarget),
      candidate(urlTarget),
    ]);
    expect(content).toContain("files  (3 tools)");
    expect(content).not.toContain("answered by");
  });

  test("no note when only one daemon matches, even with annotate on", async () => {
    const content = await discoverWithEnv({}, [candidate(socketTarget)]);
    expect(content).toContain("files  (3 tools)");
    expect(content).not.toContain("answered by");
  });
});
