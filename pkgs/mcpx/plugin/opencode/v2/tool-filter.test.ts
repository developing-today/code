/**
 * The per-session mcpx tool filter (#5122), against the same stub
 * storage and plain hook-input objects as v2.test.ts — the v2 host is
 * not runnable here, so the stubs are the contract.
 *
 *   bun test plugin/opencode/v2/tool-filter.test.ts
 */
import { describe, expect, test } from "bun:test";
import { setupMcpxV2, type McpxV2Context } from "./index.ts";
import { resolveOptions } from "./options.ts";
import type { StorageLike } from "./remember.ts";
import {
  createToolFilter,
  filterContextTools,
  readDisabledTools,
  readDisabledToolsPolicy,
  setDisabledTools,
} from "./tool-filter.ts";

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

const requestTools = () => ({
  mcpx_exec: toolDef("exec"),
  mcpx_daemon_status: toolDef("status"),
  read: toolDef("host read"),
  bash: toolDef("host bash"),
});

describe("filterContextTools", () => {
  test("hides disabled mcpx tools for the session, enabled ones remain", () => {
    const tools = requestTools();
    const removed = filterContextTools({ sessionID: "s1", tools }, new Set(["mcpx_exec"]));
    expect(removed).toEqual(["mcpx_exec"]);
    expect(Object.keys(tools).sort()).toEqual(["bash", "mcpx_daemon_status", "read"]);
  });

  test("non-mcpx tools are untouched, even when the policy names them", () => {
    const tools = requestTools();
    const removed = filterContextTools({ sessionID: "s1", tools }, new Set(["read", "bash"]));
    expect(removed).toEqual([]);
    expect(Object.keys(tools).length).toBe(4);
  });

  test("the registry is not mutated: only the per-request record changes", () => {
    // Core rebuilds the tools record from the registry for every model
    // request (model-request.ts). Filtering one request's record must
    // leave the definitions themselves — and the next request's fresh
    // record — intact.
    const registry = requestTools();
    const firstRequest = { ...registry };
    filterContextTools({ sessionID: "s1", tools: firstRequest }, new Set(["mcpx_exec"]));
    expect(registry.mcpx_exec).toBeDefined();
    const secondRequest = { ...registry };
    expect(secondRequest.mcpx_exec).toBe(registry.mcpx_exec);
    // Surviving definitions keep their identity, which core's
    // match-by-identity step relies on.
    const kept = requestTools();
    const execDef = kept.mcpx_daemon_status;
    filterContextTools({ sessionID: "s1", tools: kept }, new Set(["mcpx_exec"]));
    expect(kept.mcpx_daemon_status).toBe(execDef);
  });

  test("no tools record or empty policy is a no-op", () => {
    expect(filterContextTools({ sessionID: "s1" }, new Set(["mcpx_exec"]))).toEqual([]);
    const tools = requestTools();
    expect(filterContextTools({ sessionID: "s1", tools }, new Set())).toEqual([]);
    expect(Object.keys(tools).length).toBe(4);
  });
});

describe("disabled-tools policy in ctx.storage", () => {
  test("round-trips through set/read", async () => {
    const storage = stubStorage();
    await setDisabledTools(storage, "s1", ["mcpx_exec", "mcpx_daemon_select"]);
    expect([...(await readDisabledTools(storage, "s1"))].sort()).toEqual(["mcpx_daemon_select", "mcpx_exec"]);
    expect(await readDisabledTools(storage, "s2")).toEqual(new Set());
  });

  test("malformed records read as no policy; hostile entries are dropped", async () => {
    const storage = stubStorage();
    await storage.set("tool-filter:s1", "not-a-record");
    expect(await readDisabledTools(storage, "s1")).toEqual(new Set());
    await storage.set("tool-filter:s1", { disabled: "mcpx_exec" });
    expect(await readDisabledTools(storage, "s1")).toEqual(new Set());
    // A policy must never be able to hide a host tool: non-mcpx and
    // non-string entries are dropped on read.
    await storage.set("tool-filter:s1", { disabled: ["read", 42, null, "mcpx_exec"] });
    expect(await readDisabledTools(storage, "s1")).toEqual(new Set(["mcpx_exec"]));
  });
});

describe("createToolFilter", () => {
  test("refresh loads the session policy; apply hides only for that session", async () => {
    const storage = stubStorage();
    await setDisabledTools(storage, "s1", ["mcpx_exec"]);
    const filter = createToolFilter(storage);
    // Before any refresh there is no snapshot: nothing is hidden.
    const early = requestTools();
    expect(filter.apply({ sessionID: "s1", tools: early })).toEqual([]);
    await filter.refresh("s1");
    const tools = requestTools();
    expect(filter.apply({ sessionID: "s1", tools })).toEqual(["mcpx_exec"]);
    expect(tools.mcpx_exec).toBeUndefined();
    // Another session has no policy: its tools stay visible.
    const other = requestTools();
    expect(filter.apply({ sessionID: "s2", tools: other })).toEqual([]);
    expect(other.mcpx_exec).toBeDefined();
  });

  test("a failed refresh keeps the previous snapshot", async () => {
    const storage = stubStorage();
    await setDisabledTools(storage, "s1", ["mcpx_exec"]);
    const filter = createToolFilter(storage);
    await filter.refresh("s1");
    storage.get = async () => {
      throw new Error("storage down");
    };
    await filter.refresh("s1");
    const tools = requestTools();
    expect(filter.apply({ sessionID: "s1", tools })).toEqual(["mcpx_exec"]);
  });
});

describe("setup: the disabledTools option is the production policy writer", () => {
  // The Codex finding on PR #5170: the filter read a storage record
  // nothing in production wrote. The stub context below is the
  // tools.test.ts shape, extended so a fired context input carries a
  // tools record the hook can filter.
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
    const fireContext = (sessionID: string) => {
      const input = { sessionID, system: [] as unknown[], tools: requestTools() };
      contextFn!(input as never);
      return input;
    };
    return { ctx, fireContext };
  };

  const flush = async (): Promise<void> => {
    for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
  };

  test("setup records the option as the session policy; the next turn hides the tool", async () => {
    const storage = stubStorage();
    const { ctx, fireContext } = stubCtx(storage);
    await setupMcpxV2(ctx, {
      tools: false,
      daemonTools: false,
      disabledTools: ["mcpx_exec"],
    });
    // First turn: no snapshot exists yet, so nothing is hidden — the
    // seeding write the hook just started is what the refresh loads.
    expect(fireContext("s1").tools.mcpx_exec).toBeDefined();
    await flush();
    expect([...(await readDisabledTools(storage, "s1"))]).toEqual(["mcpx_exec"]);
    const second = fireContext("s1");
    expect(second.tools.mcpx_exec).toBeUndefined();
    expect(second.tools.mcpx_daemon_status).toBeDefined();
    expect(second.tools.read).toBeDefined();
  });

  test("an explicitly empty session policy is never overwritten by the option", async () => {
    // #5122 Codex re-review (PR #5170): { disabled: [] } is a policy
    // the session owns — its opt-out of global filtering. Seeding
    // must distinguish it from absence, or the option overwrites it.
    const storage = stubStorage();
    await setDisabledTools(storage, "s1", []);
    const { ctx, fireContext } = stubCtx(storage);
    await setupMcpxV2(ctx, {
      tools: false,
      daemonTools: false,
      disabledTools: ["mcpx_exec"],
    });
    fireContext("s1");
    await flush();
    expect([...(await readDisabledTools(storage, "s1"))]).toEqual([]);
    const second = fireContext("s1");
    expect(second.tools.mcpx_exec).toBeDefined();
    expect(second.tools.mcpx_daemon_status).toBeDefined();
  });

  test("the policy read distinguishes absence from an explicitly empty policy", async () => {
    const storage = stubStorage();
    expect(await readDisabledToolsPolicy(storage, "absent")).toBeUndefined();
    await setDisabledTools(storage, "empty", []);
    expect(await readDisabledToolsPolicy(storage, "empty")).toEqual(new Set());
    await setDisabledTools(storage, "own", ["mcpx_exec"]);
    expect(await readDisabledToolsPolicy(storage, "own")).toEqual(new Set(["mcpx_exec"]));
    await storage.set("tool-filter:malformed", { nope: true });
    expect(await readDisabledToolsPolicy(storage, "malformed")).toBeUndefined();
  });

  test("a session policy of its own is never overwritten by the option", async () => {
    const storage = stubStorage();
    await setDisabledTools(storage, "s1", ["mcpx_daemon_status"]);
    const { ctx, fireContext } = stubCtx(storage);
    await setupMcpxV2(ctx, {
      tools: false,
      daemonTools: false,
      disabledTools: ["mcpx_exec"],
    });
    fireContext("s1");
    await flush();
    expect([...(await readDisabledTools(storage, "s1"))]).toEqual(["mcpx_daemon_status"]);
    const second = fireContext("s1");
    expect(second.tools.mcpx_daemon_status).toBeUndefined();
    expect(second.tools.mcpx_exec).toBeDefined();
  });

  test("the option resolves from the array, then the env string", () => {
    expect(resolveOptions({ disabledTools: ["mcpx_exec"] }, {}).disabledTools).toEqual(["mcpx_exec"]);
    expect(resolveOptions({}, { MCPX_PLUGIN_DISABLED_TOOLS: "mcpx_exec, mcpx_daemon_status" }).disabledTools).toEqual([
      "mcpx_exec",
      "mcpx_daemon_status",
    ]);
    expect(resolveOptions({}, {}).disabledTools).toBeUndefined();
  });
});
