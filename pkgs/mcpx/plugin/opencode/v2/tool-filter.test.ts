/**
 * The per-session mcpx tool filter (#5122), against the same stub
 * storage and plain hook-input objects as v2.test.ts — the v2 host is
 * not runnable here, so the stubs are the contract.
 *
 *   bun test plugin/opencode/v2/tool-filter.test.ts
 */
import { describe, expect, test } from "bun:test";
import type { StorageLike } from "./remember.ts";
import {
  createToolFilter,
  filterContextTools,
  readDisabledTools,
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
    const removed = filterContextTools(
      { sessionID: "s1", tools },
      new Set(["mcpx_exec"]),
    );
    expect(removed).toEqual(["mcpx_exec"]);
    expect(Object.keys(tools).sort()).toEqual(["bash", "mcpx_daemon_status", "read"]);
  });

  test("non-mcpx tools are untouched, even when the policy names them", () => {
    const tools = requestTools();
    const removed = filterContextTools(
      { sessionID: "s1", tools },
      new Set(["read", "bash"]),
    );
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
    expect([...(await readDisabledTools(storage, "s1"))].sort()).toEqual([
      "mcpx_daemon_select",
      "mcpx_exec",
    ]);
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
