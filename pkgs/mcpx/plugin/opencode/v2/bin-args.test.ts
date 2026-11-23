/**
 * plugin.binArgs, observed through the real tool surface (#8472).
 *
 * The binArgs setting — MCPX_PLUGIN_BIN_ARGS — names extra arguments
 * the plugin host prepends to every invocation of the mcpx binary:
 * runBin in plugin/core/daemon.ts spawns
 * [...splitArgs(MCPX_PLUGIN_BIN_ARGS), ...argv] whenever a tool call
 * takes the binary path (backend=cli always, backend=auto whenever no
 * daemon answers). The choice lives entirely inside this plugin host,
 * so this suite is where its effect can be observed. Like bin.test.ts,
 * the runtime here is NOT given a runBin stub: the real runBin spawns
 * the configured executable, and the executable is a stub script whose
 * only job is to print the argv it was handed, one argument per line,
 * so an argument containing a space is distinguishable from two
 * arguments. The Go effect probe for the setting
 * (TestPluginBinArgsPrependsConfiguredArgumentsFromTheEnvironment in
 * internal/e2e) runs this file. The cases set the setting the way a
 * user would — MCPX_PLUGIN_BIN_ARGS through resolveOptions — and
 * assert the observable with/without difference the effect guard
 * (#1971) demands: the configured arguments arrive ahead of the
 * tool's own argv, in both the whitespace and JSON spellings, and no
 * extra arguments arrive when the setting is unset.
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

const recorder = join(dir, "mcpx-recorder");
writeFileSync(recorder, `#!/bin/sh\nprintf 'argv<<%s>>\\n' "$@"\n`);
chmodSync(recorder, 0o755);

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

const argvOf = (content: string): string[] =>
  content
    .split("\n")
    .filter((l) => l.startsWith("argv<<") && l.endsWith(">>"))
    .map((l) => l.slice("argv<<".length, -">>".length));

describe("plugin.binArgs (MCPX_PLUGIN_BIN_ARGS)", () => {
  test("the whitespace form is split and prepended ahead of the tool's own argv", async () => {
    const content = await discoverWithEnv({
      MCPX_PLUGIN_BACKEND: "cli",
      MCPX_PLUGIN_BIN: recorder,
      MCPX_PLUGIN_BIN_ARGS: "--ju-prelude alpha",
    });
    expect(argvOf(content)).toEqual(["--ju-prelude", "alpha", "ls"]);
  });

  test("the JSON form keeps an argument containing a space in one piece", async () => {
    const content = await discoverWithEnv({
      MCPX_PLUGIN_BACKEND: "cli",
      MCPX_PLUGIN_BIN: recorder,
      MCPX_PLUGIN_BIN_ARGS: "[\"--ju-flag\",\"two words\"]",
    });
    expect(argvOf(content)).toEqual(["--ju-flag", "two words", "ls"]);
  });

  test("with the setting unset the executable receives the tool's argv and nothing else", async () => {
    const content = await discoverWithEnv({
      MCPX_PLUGIN_BACKEND: "cli",
      MCPX_PLUGIN_BIN: recorder,
    });
    expect(argvOf(content)).toEqual(["ls"]);
  });

  test("a different value alone changes what arrives — the setting decides", async () => {
    const content = await discoverWithEnv({
      MCPX_PLUGIN_BACKEND: "cli",
      MCPX_PLUGIN_BIN: recorder,
      MCPX_PLUGIN_BIN_ARGS: "--other",
    });
    expect(argvOf(content)).toEqual(["--other", "ls"]);
    expect(content).not.toContain("--ju-prelude");
  });
});
