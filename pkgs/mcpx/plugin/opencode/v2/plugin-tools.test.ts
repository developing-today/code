/**
 * plugin.tools, observed through the real registration surface (#8472).
 *
 * The tools setting — MCPX_PLUGIN_TOOLS — is the opt-in that puts
 * mcpx's own tool set (discover / exec / observe) in front of the
 * model. resolveOptions reads it from the environment (truthy
 * spellings: 1 / true / yes / on; default off), setup passes it to
 * registerTools as toolsEnabled (v2/index.ts), and registerTools
 * only adds the tool set when it is on. The setting also feeds the
 * daemon tools' default: daemonToolsWanted treats tools === true
 * as "offer the daemon tools too", so turning the tool set on
 * brings daemon_status / daemon_select / daemon_pick /
 * daemon_forget with it, while off (and an unambiguous discovery)
 * registers nothing but the namespace. The setting lives entirely
 * inside this plugin host, so this suite is where its effect can
 * be observed: the runtime is the real createToolRuntime with a
 * stub discovery (no candidates, not ambiguous — the boot state
 * registerTools consults for the daemon-tools default), and the
 * editor is a recording stub in the shape tools.test.ts uses. The
 * Go effect probe for the setting
 * (TestPluginToolsRegistersTheToolSetFromTheEnvironment in
 * internal/e2e) runs this file. The cases set the setting the way
 * a user would — MCPX_PLUGIN_TOOLS through resolveOptions — and
 * assert the observable with/without differences the effect guard
 * (#1971) demands: unset registers no tools; true registers the
 * full set plus the daemon tools; an explicit false registers no
 * tools, exactly like unset; a different truthy value yields the
 * same full set — the setting alone decides.
 *
 *   bun test plugin/opencode/v2/plugin-tools.test.ts
 */
import { describe, expect, test } from "bun:test";
import type { Discovery } from "../../core/daemon.ts";
import { resolveOptions } from "./options.ts";
import type { StorageLike } from "./remember.ts";
import {
  createToolRuntime,
  registerTools,
  type McpxToolInfo,
  type ToolDomainLike,
  type ToolEditorLike,
} from "./tools.ts";

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

// Registration exactly as setup performs it (v2/index.ts): the
// resolved options decide toolsEnabled and the daemon-tools
// tri-state; the recording editor is what the host would mutate.
const registerWithEnv = async (env: Record<string, string | undefined>) => {
  const options = resolveOptions(undefined, env);
  const rt = createToolRuntime({
    directory: "/tmp",
    env,
    options,
    storage: stubStorage(),
    discoverFn: async () => noDiscovery,
    assertPermission: async () => {},
  } as never);
  const editor = stubEditor();
  const domain: ToolDomainLike = {
    transform: async (fn) => {
      fn(editor);
      return {};
    },
  };
  const names = await registerTools(domain, rt, {
    toolsEnabled: options.tools ?? false,
    daemonTools: options.daemonTools,
  });
  return { names, editor };
};

const FULL_SET = [
  "mcpx_discover",
  "mcpx_exec",
  "mcpx_observe",
  "mcpx_daemon_status",
  "mcpx_daemon_select",
  "mcpx_daemon_pick",
  "mcpx_daemon_forget",
];

describe("plugin.tools (MCPX_PLUGIN_TOOLS)", () => {
  test("unset: no tools are registered, only the namespace", async () => {
    const { names, editor } = await registerWithEnv({});
    expect(names).toEqual([]);
    expect(editor.added).toEqual([]);
    expect(editor.namespaces.map((n) => n.name)).toEqual(["mcpx"]);
  });

  test("true: the full tool set is registered, daemon tools included", async () => {
    const { names, editor } = await registerWithEnv({ MCPX_PLUGIN_TOOLS: "true" });
    expect(names).toEqual(FULL_SET);
    expect(editor.added.map((t) => t.name)).toEqual([
      "discover",
      "exec",
      "observe",
      "daemon_status",
      "daemon_select",
      "daemon_pick",
      "daemon_forget",
    ]);
  });

  test("explicit false: no tools are registered, exactly like unset", async () => {
    const { names, editor } = await registerWithEnv({ MCPX_PLUGIN_TOOLS: "false" });
    expect(names).toEqual([]);
    expect(editor.added).toEqual([]);
  });

  test("a different truthy value yields the same full set — the setting alone decides", async () => {
    const { names } = await registerWithEnv({ MCPX_PLUGIN_TOOLS: "1" });
    expect(names).toEqual(FULL_SET);
  });
});
