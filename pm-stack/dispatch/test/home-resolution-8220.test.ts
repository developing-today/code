// The dispatch side of the #5762 home-resolution contract (#8220): every
// default under HOME resolves exactly as before when HOME is set, an unset or
// empty HOME is HomeUnresolvedError naming the setting that needed it, and an
// explicit override never needs HOME.
import { afterEach, describe, expect, test } from "bun:test";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { HomeUnresolvedError, requireHome } from "../src/home.ts";

const SAVED_HOME = process.env.HOME;
afterEach(() => {
  if (SAVED_HOME === undefined) delete process.env.HOME;
  else process.env.HOME = SAVED_HOME;
});

/** Each config default under HOME: its override variable, its config key and its path under HOME. */
const HOME_DEFAULTS = [
  ["QUEUE_BROKER_TOKEN_FILE", "queueBrokerTokenFile", [".config", "queue-broker", "token"]],
  ["OPENCODE_AUTH_FILE", "opencodeAuthFile", [".local", "state", "opencode", "service-prod.json"]],
  ["PM_DISPATCH_WORKTREE_ROOT", "worktreeRoot", ["git", "fleet"]],
  [
    "PM_DISPATCH_WORKTREE_REGISTRY",
    "worktreeRegistryFile",
    [".local", "state", "pm-dispatch", "worktree-registry.json"],
  ],
  ["QUOTA_CURRENT_PATH", "quotaCurrentFile", [".local", "state", "pm-aggregator", "quota-current.json"]],
] as const;

/** Every override set, so loadConfig needs HOME for none of them. */
const ALL_OVERRIDES = Object.fromEntries(HOME_DEFAULTS.map(([v]) => [v, `/override/${v}`]));

describe("dispatch home resolution (#8220)", () => {
  test("a set HOME gives the same default paths as before", () => {
    process.env.HOME = "/home/probe";
    const config = loadConfig({}) as unknown as Record<string, unknown>;
    for (const [, key, segments] of HOME_DEFAULTS) {
      expect(config[key]).toBe(join("/home/probe", ...segments));
    }
  });

  for (const home of [undefined, "", "   "]) {
    const label = home === undefined ? "unset" : JSON.stringify(home);
    test(`HOME ${label} names the first setting that needed it`, () => {
      if (home === undefined) delete process.env.HOME;
      else process.env.HOME = home;
      expect(() => loadConfig({})).toThrow(HomeUnresolvedError);
      try {
        loadConfig({});
      } catch (e) {
        expect((e as HomeUnresolvedError).setting).toBe("QUEUE_BROKER_TOKEN_FILE");
        expect((e as Error).message).toContain("set HOME or set QUEUE_BROKER_TOKEN_FILE");
        expect((e as Error).message).toContain(home === undefined ? "HOME is unset" : "HOME is empty");
      }
    });

    for (const [variable] of HOME_DEFAULTS) {
      test(`HOME ${label}: only ${variable} unset names ${variable}`, () => {
        if (home === undefined) delete process.env.HOME;
        else process.env.HOME = home;
        const env: Record<string, string> = { ...ALL_OVERRIDES };
        delete env[variable];
        expect(() => loadConfig(env)).toThrow(`set HOME or set ${variable}`);
      });
    }
  }

  test("explicit overrides never need HOME", () => {
    delete process.env.HOME;
    const config = loadConfig({ ...ALL_OVERRIDES }) as unknown as Record<string, unknown>;
    for (const [variable, key] of HOME_DEFAULTS) {
      expect(config[key]).toBe(`/override/${variable}`);
    }
  });

  test("a worktree root of off needs no HOME for it", () => {
    delete process.env.HOME;
    const env: Record<string, string> = { ...ALL_OVERRIDES, PM_DISPATCH_WORKTREE_ROOT: "off" };
    expect(loadConfig(env).worktreeRoot).toBeNull();
  });

  test("requireHome returns a set HOME unchanged", () => {
    expect(requireHome("X", { HOME: "/home/probe" })).toBe("/home/probe");
    expect(() => requireHome("the fleet binary", {})).toThrow("set HOME (home-resolution");
  });
});
