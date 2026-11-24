/**
 * plugin.binArgs, observed through the real tool surface (#8472).
 *
 * The binArgs setting — MCPX_PLUGIN_BIN_ARGS — names extra
 * arguments the plugin host prepends to every spawn of the mcpx
 * binary: whenever a tool call takes the binary path (backend=cli
 * always, backend=auto when no daemon answers), runBin in
 * plugin/core/daemon.ts runs the executable with
 * [...binArgs, ...argv]. The setting lives entirely inside this
 * plugin host (resolveOptions reads it from the environment;
 * splitArgs takes a JSON array when the value starts with "[" so
 * an argument containing a space can be expressed exactly, and
 * splits on whitespace otherwise), so this suite is where its
 * effect can be observed. As in bin.test.ts, the runtime here is
 * NOT given a runBin stub: the real runBin spawns the configured
 * executable, and the executable is a stub script whose only job
 * is to read back the argv it was handed, one argument per line,
 * so a space inside one argument is distinguishable from two
 * arguments. The Go effect probe for the setting
 * (TestPluginBinArgsPrependsTheConfiguredArgumentsFromTheEnvironment
 * in internal/e2e) runs this file. The cases set the setting the
 * way a user would — MCPX_PLUGIN_BIN_ARGS through resolveOptions —
 * and assert the observable with/without differences the effect
 * guard (#1971) demands: no setting, no extra arguments; the
 * whitespace form prepends each word in order ahead of the tool's
 * own argv; the JSON form keeps a space inside one argument; a
 * different value yields that value's arguments instead.
 *
 *   bun test plugin/opencode/v2/bin-args.test.ts
 */
import { afterAll, describe, expect, test } from "bun:test";
import { chmodSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Discovery } from "../../core/daemon.ts";
import { resolveOptions } from "./options.ts";
import type { StorageLike } from "./remember.ts";
import { createToolRuntime, type DaemonClientLike } from "./tools.ts";

const dir = mkdtempSync(join(tmpdir(), "mcpx-plugin-bin-args-"));

const stubPath = join(dir, "mcpx-stub");
writeFileSync(stubPath, `#!/bin/sh\necho "stub-argv:"\nfor a in "$@"; do echo "arg: $a"; done\n`);
chmodSync(stubPath, 0o755);

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

// The argv the stub executable was handed, in order: the discover
// tool's own binary-path argv is ["ls"], so anything ahead of it
// arrived via plugin.binArgs.
const argvOf = (content: string): string[] =>
  content
    .split("\n")
    .filter((line) => line.startsWith("arg: "))
    .map((line) => line.slice("arg: ".length));

describe("plugin.binArgs (MCPX_PLUGIN_BIN_ARGS)", () => {
  test("unset: the executable sees only the tool's own argv", async () => {
    const content = await discoverWithEnv({
      MCPX_PLUGIN_BACKEND: "cli",
      MCPX_PLUGIN_BIN: stubPath,
    });
    expect(argvOf(content)).toEqual(["ls"]);
  });

  test("whitespace form: each word is prepended in order ahead of the tool's argv", async () => {
    const content = await discoverWithEnv({
      MCPX_PLUGIN_BACKEND: "cli",
      MCPX_PLUGIN_BIN: stubPath,
      MCPX_PLUGIN_BIN_ARGS: "--config /tmp/mcpx-stub-config --verbose",
    });
    expect(argvOf(content)).toEqual(["--config", "/tmp/mcpx-stub-config", "--verbose", "ls"]);
  });

  test("JSON form: an argument containing a space arrives as one argv element", async () => {
    const content = await discoverWithEnv({
      MCPX_PLUGIN_BACKEND: "cli",
      MCPX_PLUGIN_BIN: stubPath,
      MCPX_PLUGIN_BIN_ARGS: '["--greeting", "hello world"]',
    });
    expect(argvOf(content)).toEqual(["--greeting", "hello world", "ls"]);
  });

  test("a different value yields that value's arguments instead — the setting alone decides", async () => {
    const content = await discoverWithEnv({
      MCPX_PLUGIN_BACKEND: "cli",
      MCPX_PLUGIN_BIN: stubPath,
      MCPX_PLUGIN_BIN_ARGS: "--other",
    });
    expect(argvOf(content)).toEqual(["--other", "ls"]);
  });
});
