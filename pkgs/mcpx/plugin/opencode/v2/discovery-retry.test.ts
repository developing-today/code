/**
 * plugin.discoveryRetry, observed through the real tool runtime (#8472).
 *
 * The discoveryRetry setting — MCPX_PLUGIN_DISCOVERY_RETRY, a Go
 * duration — is how long the plugin waits before looking for mcpx
 * again after a discovery miss, so an editor without mcpx does not
 * pay for a failed ladder run on every tool call. The setting lives
 * entirely inside this plugin host: resolveOptions parses the env
 * value with goDurationMs into discoveryRetryMs, and the tool
 * runtime's look() in tools.ts holds a miss for that long — a look
 * inside the window answers the rung-6 "none" discovery without
 * running the ladder again, and only once the window has passed
 * does the next look rediscover. This suite drives the real
 * runtime with a counting discoverFn that always misses, so the
 * ladder runs are the observable: the cases set the setting the way
 * a user would — MCPX_PLUGIN_DISCOVERY_RETRY through
 * resolveOptions — and assert the with/without differences the
 * effect guard (#1971) demands: unset keeps the built-in 60s
 * cooldown (TUNING.missCooldownMs); a long value holds the miss
 * just the same; 0s disables the hold so every look rediscovers;
 * a short value holds inside its window and releases after it; an
 * unparseable value resolves to nothing and the default applies.
 * The Go effect probe for the setting
 * (TestPluginDiscoveryRetryDelaysRediscoveryFromTheEnvironment in
 * internal/e2e) runs this file.
 *
 *   bun test plugin/opencode/v2/discovery-retry.test.ts
 */
import { describe, expect, test } from "bun:test";
import type { Discovery } from "../../core/daemon.ts";
import { resolveOptions } from "./options.ts";
import type { StorageLike } from "./remember.ts";
import { createToolRuntime } from "./tools.ts";

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

// A runtime whose ladder always misses, with the setting resolved
// from the given environment the way the host resolves it. The
// returned counter is how many times the ladder actually ran.
const missingRuntime = (env: Record<string, string | undefined>) => {
  let ladderRuns = 0;
  const rt = createToolRuntime({
    directory: "/work",
    env,
    options: resolveOptions(undefined, env),
    storage: stubStorage(),
    discoverFn: async () => {
      ladderRuns += 1;
      return noDiscovery;
    },
  } as never);
  return { rt, ladderRuns: () => ladderRuns };
};

describe("plugin.discoveryRetry (MCPX_PLUGIN_DISCOVERY_RETRY)", () => {
  test("unset: the default cooldown holds a miss — the second look does not rediscover", async () => {
    const { rt, ladderRuns } = missingRuntime({});
    const first = await rt.look();
    expect(first.target).toBeUndefined();
    expect(ladderRuns()).toBe(1);
    const second = await rt.look();
    expect(ladderRuns()).toBe(1);
    expect(second).toEqual({
      candidates: [],
      rung: 6,
      reason: "none",
      ambiguous: false,
    });
  });

  test("a long value holds the miss: repeated looks inside the window never rediscover", async () => {
    const { rt, ladderRuns } = missingRuntime({
      MCPX_PLUGIN_DISCOVERY_RETRY: "1h",
    });
    await rt.look();
    await rt.look();
    await rt.look();
    expect(ladderRuns()).toBe(1);
  });

  test("0s disables the hold: every look runs the ladder again", async () => {
    const { rt, ladderRuns } = missingRuntime({
      MCPX_PLUGIN_DISCOVERY_RETRY: "0s",
    });
    await rt.look();
    await rt.look();
    await rt.look();
    expect(ladderRuns()).toBe(3);
  });

  test("a short value holds inside its window and releases after it", async () => {
    const { rt, ladderRuns } = missingRuntime({
      MCPX_PLUGIN_DISCOVERY_RETRY: "50ms",
    });
    await rt.look();
    await rt.look();
    expect(ladderRuns()).toBe(1);
    await new Promise((resolve) => setTimeout(resolve, 90));
    await rt.look();
    expect(ladderRuns()).toBe(2);
  });

  test("an unparseable value resolves to the default cooldown, not to zero", async () => {
    const { rt, ladderRuns } = missingRuntime({
      MCPX_PLUGIN_DISCOVERY_RETRY: "whenever",
    });
    await rt.look();
    await rt.look();
    expect(ladderRuns()).toBe(1);
  });
});
