/**
 * Core option resolution, against the v1 semantics the resolver
 * documents (oracle: the resolveOptions contract in options.ts --
 * plugin option first, then the environment variable, then the
 * caller's default; an explicit empty bin is a decision to forbid
 * spawning, never an absence; an unrecognised truthy value is
 * undefined, so the default applies).
 *
 *   bun test plugin/core/options.test.ts
 */
import { describe, expect, test } from "bun:test";
import { resolveOptions, truthy } from "./options.ts";

describe("truthy (oracle: v1 truthy parse)", () => {
  test("recognised spellings parse, unrecognised is undefined", () => {
    expect(truthy("1")).toBe(true);
    expect(truthy("off")).toBe(false);
    expect(truthy(true)).toBe(true);
    expect(truthy("maybe")).toBeUndefined();
    expect(truthy("")).toBeUndefined();
    expect(truthy(undefined)).toBeUndefined();
  });
});

describe("resolveOptions (oracle: option > env > default)", () => {
  test("defaults land when nothing is set", () => {
    const r = resolveOptions(undefined, {});
    expect(r.backend).toBe("auto");
    expect(r.remember).toBe("session");
    expect(r.annotate).toBe(true);
    expect(r.tools).toBe(false);
    expect(r.instructions).toBe(false);
    expect(r.toolTiming).toBe(false);
    expect(r.answerSampling).toBe(true);
    expect(r.endpoint).toBeUndefined();
    expect(r.bin).toBeUndefined();
  });

  test("env beats default, option beats env", () => {
    const env = { MCPX_PLUGIN_BACKEND: "cli", MCPX_DAEMON_ENDPOINT: "unix:///tmp/x" };
    expect(resolveOptions(undefined, env).backend).toBe("cli");
    expect(resolveOptions({ backend: "v1" }, env).backend).toBe("v1");
    expect(resolveOptions(undefined, env).endpoint).toBe("unix:///tmp/x");
  });

  test("an explicit empty bin forbids spawning and survives resolution", () => {
    expect(resolveOptions({ bin: "" }, { MCPX_PLUGIN_BIN: "/usr/bin/mcpx" }).bin).toBe("");
    expect(resolveOptions(undefined, { MCPX_PLUGIN_BIN: "/usr/bin/mcpx" }).bin).toBe("/usr/bin/mcpx");
  });

  test("binArgs strings split, arrays pass through", () => {
    expect(resolveOptions({ binArgs: "--flag value" }, {}).binArgs).toEqual(["--flag", "value"]);
    expect(resolveOptions({ binArgs: ["a"] }, {}).binArgs).toEqual(["a"]);
  });

  test("disabledTools parses arrays and separated strings", () => {
    expect(resolveOptions({ disabledTools: ["mcpx_exec"] }, {}).disabledTools).toEqual(["mcpx_exec"]);
    expect(resolveOptions(undefined, { MCPX_PLUGIN_DISABLED_TOOLS: "mcpx_exec, mcpx_search" }).disabledTools).toEqual([
      "mcpx_exec",
      "mcpx_search",
    ]);
  });
});
