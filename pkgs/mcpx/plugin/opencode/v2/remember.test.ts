/**
 * plugin.remember, observed through the real tool surface (#8472).
 *
 * The remember setting — MCPX_PLUGIN_REMEMBER — names how long a
 * daemon choice stays chosen: "session" (the default) keeps it in
 * memory only, while "until-gone" and "indefinite" write it to the
 * host's per-plugin storage (remember.ts, under the key
 * daemon:<directory>), where a later session's warm read recalls
 * it. The setting lives entirely inside this plugin host
 * (resolveOptions reads it from the environment; daemon_pick falls
 * back to it when a call names no scope of its own), so this suite
 * is where its effect can be observed. The runtime here is the
 * real one — real tools, the real picker, the real storage-backed
 * applyChoice — with only discovery, the permission prompt and
 * the daemon client stubbed, and the cases set the setting the
 * way a user would — MCPX_PLUGIN_REMEMBER through resolveOptions —
 * and assert the observable with/without differences the effect
 * guard (#1971) demands: unset writes nothing to storage and the
 * note says the choice is for this session only; until-gone and
 * indefinite each persist the picked target under the directory's
 * key, readable back through the real readRememberedDaemon, with
 * the note naming that lifetime; and the scope shown to the
 * permission prompt is the setting's value. The Go effect probe
 * for the setting
 * (TestPluginRememberScopePersistsTheDaemonChoiceFromTheEnvironment
 * in internal/e2e) runs this file.
 *
 *   bun test plugin/opencode/v2/remember.test.ts
 */
import { afterAll, describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { rank, targetKey, type Candidate, type Discovery, type Target } from "../../core/daemon.ts";
import { resolveOptions } from "./options.ts";
import { readRememberedDaemon, type StorageLike } from "./remember.ts";
import { createToolRuntime, type DaemonClientLike } from "./tools.ts";

const dir = mkdtempSync(join(tmpdir(), "mcpx-plugin-remember-"));
afterAll(() => rmSync(dir, { recursive: true, force: true }));

const picked: Target = { kind: "socket", path: "/tmp/mcpx-remember-a.sock" };
const other: Target = { kind: "socket", path: "/tmp/mcpx-remember-b.sock" };

const candidate = (target: Target): Candidate => ({
  target,
  source: "info-file",
  configPath: `/cfg/${target.path.split("/").pop()}.json`,
});

const discovery: Discovery = {
  target: picked,
  chosen: undefined,
  candidates: [candidate(picked), candidate(other)],
  rung: 1,
  reason: "info-file",
  ambiguous: false,
};

const stubStorage = () => {
  const map = new Map<string, unknown>();
  const storage: StorageLike = {
    get: async (k) => map.get(k),
    set: async (k, v) => void map.set(k, v),
    remove: async (k) => void map.delete(k),
  };
  return { storage, map };
};

const pickWithEnv = async (env: Record<string, string | undefined>) => {
  const { storage, map } = stubStorage();
  const asserted: { metadata?: { remember?: unknown } }[] = [];
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
    storage,
    discoverFn: async () => discovery,
    clientFor: () => fakeClient(),
    assertPermission: async (input: never) => void asserted.push(input),
  } as never);
  const pick = rt.daemonTools().find((t) => t.name === "daemon_pick")!;
  const idx = rank(discovery.candidates, dir).findIndex((c) => targetKey(c.target) === targetKey(picked));
  const out = await pick.execute({ candidate: idx } as never, { sessionID: "s1" });
  return { content: out.content, map, storage, asserted };
};

describe("plugin.remember (MCPX_PLUGIN_REMEMBER)", () => {
  test("unset: the default is session — the choice is kept in memory and nothing is written to storage", async () => {
    const { content, map, storage, asserted } = await pickWithEnv({});
    expect(content).toContain("Remembered for this session only.");
    expect(asserted[0]?.metadata?.remember).toBe("session");
    expect(map.size).toBe(0);
    expect(await readRememberedDaemon(storage, dir)).toBeUndefined();
  });

  test("session: an explicit session scope persists nothing either", async () => {
    const { content, map, storage } = await pickWithEnv({ MCPX_PLUGIN_REMEMBER: "session" });
    expect(content).toContain("Remembered for this session only.");
    expect(map.size).toBe(0);
    expect(await readRememberedDaemon(storage, dir)).toBeUndefined();
  });

  test("until-gone: the picked target is written to storage under the directory's key", async () => {
    const { content, storage, asserted } = await pickWithEnv({ MCPX_PLUGIN_REMEMBER: "until-gone" });
    expect(content).toContain("Remembered until this daemon stops answering.");
    expect(asserted[0]?.metadata?.remember).toBe("until-gone");
    const record = await readRememberedDaemon(storage, dir);
    expect(record?.target).toEqual(picked);
    expect(record?.socket).toBe(picked.path);
    expect(typeof record?.seenAt).toBe("number");
  });

  test("indefinite: the picked target is written to storage, with the indefinite note", async () => {
    const { content, storage, asserted } = await pickWithEnv({ MCPX_PLUGIN_REMEMBER: "indefinite" });
    expect(content).toContain("Remembered indefinitely for this directory.");
    expect(asserted[0]?.metadata?.remember).toBe("indefinite");
    const record = await readRememberedDaemon(storage, dir);
    expect(record?.target).toEqual(picked);
    expect(record?.target).not.toEqual(other);
  });
});
