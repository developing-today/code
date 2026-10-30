/**
 * The v2 first slice, against stub contexts: a Map-backed storage, a fake
 * fetch for the environment route, and plain hook input objects. The v2
 * host is not runnable here, so the stubs are the contract — each one is
 * shaped after the verified facts in docs/opencode-plugin-v2.md.
 *
 *   bun test plugin/opencode/v2/v2.test.ts
 */
import { describe, expect, test } from "bun:test";
import { appendContextOnce, CONTEXT_MARKER } from "./context.ts";
import {
  forgetDaemon,
  readRememberedDaemon,
  recallDaemon,
  rememberDaemon,
  type StorageLike,
} from "./remember.ts";
import { createSessionEnv, type FetchLike } from "./session-env.ts";

const stubStorage = (): StorageLike & { map: Map<string, unknown> } => {
  const map = new Map<string, unknown>();
  return {
    map,
    get: async (k) => map.get(k),
    set: async (k, v) => void map.set(k, v),
    remove: async (k) => void map.delete(k),
  };
};

describe("remember (ctx.storage)", () => {
  test("round-trips a socket target and forgets it", async () => {
    const storage = stubStorage();
    await rememberDaemon(storage, "/work", { kind: "socket", path: "/tmp/m.sock" }, 42);
    const read = await readRememberedDaemon(storage, "/work");
    expect(read?.seenAt).toBe(42);
    expect(read?.socket).toBe("/tmp/m.sock");
    await forgetDaemon(storage, "/work");
    expect(await readRememberedDaemon(storage, "/work")).toBeUndefined();
  });

  test("until-gone: a failed health check drops the record", async () => {
    const storage = stubStorage();
    await rememberDaemon(storage, "/work", { kind: "url", base: "http://127.0.0.1:9" });
    const recalled = await recallDaemon(storage, "/work", async () => false);
    expect(recalled).toBeUndefined();
    expect(await readRememberedDaemon(storage, "/work")).toBeUndefined();
  });

  test("malformed records read as absent", async () => {
    const storage = stubStorage();
    await storage.set("daemon:/work", "not-a-record");
    expect(await readRememberedDaemon(storage, "/work")).toBeUndefined();
  });
});

describe("session environment", () => {
  test("sends the FULL environment, once per session", async () => {
    const calls: { url: string; body: string }[] = [];
    const fetchFn: FetchLike = async (url, init) => {
      calls.push({ url, body: init.body ?? "" });
      return { ok: true, status: 204 };
    };
    const adapter = createSessionEnv({
      serverUrl: "http://host:1234/",
      baseEnv: { PATH: "/bin", HOME: "/home/x", UNSET: undefined },
      fetchFn,
    });
    expect(await adapter.ensure("s 1", { MCPX_SESSION_ID: "s 1" })).toBe(true);
    expect(await adapter.ensure("s 1", { MCPX_SESSION_ID: "s 1" })).toBe(false);
    expect(calls.length).toBe(1);
    expect(calls[0]?.url).toBe("http://host:1234/api/session/s%201/environment");
    const sent = JSON.parse(calls[0]?.body ?? "{}") as {
      variables: Record<string, string>;
    };
    // The route replaces the session env: PATH/HOME must ride along.
    expect(sent.variables.PATH).toBe("/bin");
    expect(sent.variables.HOME).toBe("/home/x");
    expect(sent.variables.MCPX_SESSION_ID).toBe("s 1");
    expect("UNSET" in sent.variables).toBe(false);
  });

  test("no serverUrl degrades quietly and does not retry-storm", async () => {
    const adapter = createSessionEnv({ serverUrl: undefined, baseEnv: {} });
    expect(await adapter.ensure("s1", {})).toBe(false);
    expect(adapter.seen("s1")).toBe(true);
  });
});

describe("context paragraph", () => {
  test("appends once, however many turns fire the hook", () => {
    const input = { sessionID: "s1", system: [] as { type?: string; text?: string }[] };
    expect(appendContextOnce(input)).toBe(true);
    expect(appendContextOnce(input)).toBe(false);
    expect(input.system.length).toBe(1);
    expect(input.system[0]?.text).toContain(CONTEXT_MARKER);
  });
});
