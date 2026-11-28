/**
 * plugin.bin, observed through the real tool surface (#8472).
 *
 * The bin setting — MCPX_PLUGIN_BIN — names the executable the
 * plugin host spawns whenever a tool call takes the binary path:
 * backend=cli always, backend=auto whenever no daemon answers. The
 * choice lives entirely inside this plugin host (runBin in
 * plugin/core/daemon.ts resolves opts.bin, then MCPX_PLUGIN_BIN,
 * then the built-in default), so this suite is where its effect can
 * be observed. Unlike backend.test.ts, the runtime here is NOT given
 * a runBin stub: the real runBin spawns the configured executable,
 * and the executables are stub scripts whose only job is to say
 * their name and the argv they were handed. The Go effect probe for
 * the setting (TestPluginBinInvokesTheConfiguredExecutableFromTheEnvironment
 * in internal/e2e) runs this file. The cases set the setting the way
 * a user would — MCPX_PLUGIN_BIN through resolveOptions — and assert
 * the observable with/without difference the effect guard (#1971)
 * demands: which executable answered, and that a path that names
 * nothing runnable answers with the no-binary refusal instead of
 * silently running something else.
 *
 *   bun test plugin/opencode/v2/bin.test.ts
 */
import { afterAll, describe, expect, test } from "bun:test";
import { chmodSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Discovery } from "../../core/daemon.ts";
import { resolveOptions } from "./options.ts";
import type { StorageLike } from "./remember.ts";
import { createToolRuntime, type DaemonClientLike } from "./tools.ts";

const dir = mkdtempSync(join(tmpdir(), "mcpx-plugin-bin-"));

const stubBin = (name: string, marker: string): string => {
  const path = join(dir, name);
  writeFileSync(path, `#!/bin/sh\necho "${marker} answering: $@"\n`);
  chmodSync(path, 0o755);
  return path;
};

const alpha = stubBin("mcpx-alpha", "stub-alpha");
const beta = stubBin("mcpx-beta", "stub-beta");
const missing = join(dir, "mcpx-not-here");

afterAll(() => rmSync(dir, { recursive: true, force: true }));

const noDiscovery: Discovery = {
  target: undefined,
  chosen: undefined,
  candidates: [],
  rung: 6,
  reason: "none",
  ambiguous: false,
};

const stubStorage = (): StorageLike => {
  const map = new Map<string, unknown>();
  return {
    get: async (k) => map.get(k),
    set: async (k, v) => void map.set(k, v),
    remove: async (k) => void map.delete(k),
  };
};

const discoverWithEnv = async (env: Record<string, string | undefined>) => {
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
  const rt = createToolRuntime({
    directory: dir,
    env,
    options: resolveOptions(undefined, env),
    storage: stubStorage(),
    discoverFn: async () => noDiscovery,
    clientFor: () => fakeClient(),
    assertPermission: async () => {},
  } as never);
  const discover = rt.tools().find((t) => t.name === "discover")!;
  const out = await discover.execute({} as never, { sessionID: "s1" });
  return out.content;
};

describe("plugin.bin (MCPX_PLUGIN_BIN)", () => {
  test("cli runs the configured executable: alpha answers when MCPX_PLUGIN_BIN names alpha", async () => {
    const content = await discoverWithEnv({
      MCPX_PLUGIN_BACKEND: "cli",
      MCPX_PLUGIN_BIN: alpha,
    });
    expect(content).toContain("stub-alpha answering:");
    expect(content).not.toContain("stub-beta");
  });

  test("pointing the same setting at beta makes beta answer instead — the setting alone decides", async () => {
    const content = await discoverWithEnv({
      MCPX_PLUGIN_BACKEND: "cli",
      MCPX_PLUGIN_BIN: beta,
    });
    expect(content).toContain("stub-beta answering:");
    expect(content).not.toContain("stub-alpha");
  });

  test("auto with no daemon reachable falls back through the same configured executable", async () => {
    const content = await discoverWithEnv({ MCPX_PLUGIN_BIN: alpha });
    expect(content).toContain("stub-alpha answering:");
  });

  test("a configured path that names nothing runnable yields the no-binary refusal, not a silent substitute", async () => {
    const content = await discoverWithEnv({
      MCPX_PLUGIN_BACKEND: "cli",
      MCPX_PLUGIN_BIN: missing,
    });
    expect(content).toContain("no mcpx binary could be run");
    expect(content).not.toContain("answering:");
  });
});
