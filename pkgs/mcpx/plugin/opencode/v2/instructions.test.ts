/**
 * plugin.instructions, observed through the real v2 setup (#8472).
 *
 * The instructions setting — MCPX_PLUGIN_INSTRUCTIONS, off by
 * default — is the opt-in that adds mcpx's session-context
 * paragraph to the model's system prompt: setup's session
 * "context" hook calls appendContextOnce when the resolved option
 * is on (v2/index.ts), and the paragraph rides every request once
 * present, so it is paid help and stays off unless asked for.
 * resolveOptions reads it host option first, then the environment
 * (truthy spellings: 1 / true / yes / on), then the default false.
 * The v1 system transform is covered by
 * plugin/opencode/mcpx/instructions.test.ts; this suite covers the
 * v2 path the same setting now drives. The setting lives entirely
 * inside this plugin host, so this suite is where its effect can
 * be observed: setup is the real setupMcpxV2 with the stub context
 * shape tool-filter.test.ts uses (Map-backed storage, a captured
 * context hook, a no-op tool domain), and the cases fire the hook
 * with plain inputs and read the system array the host would send.
 * The Go effect probe for the setting
 * (TestPluginInstructionsAppendsTheContextParagraphFromTheEnvironment
 * in internal/e2e) runs this file. The cases set the setting the
 * way a user would — MCPX_PLUGIN_INSTRUCTIONS through
 * resolveOptions, or the host option — and assert the observable
 * with/without differences the effect guard (#1971) demands:
 * unset and explicit false append nothing; true appends the
 * paragraph, once per system array no matter how often the hook
 * fires; another truthy spelling appends it too; and the host
 * option beats the environment in both directions.
 *
 *   bun test plugin/opencode/v2/instructions.test.ts
 */
import { afterEach, describe, expect, test } from "bun:test";
import { CONTEXT_MARKER, CONTEXT_PARAGRAPH } from "./context.ts";
import { setupMcpxV2, type McpxV2Context } from "./index.ts";
import type { Options } from "./options.ts";
import type { StorageLike } from "./remember.ts";

const ENV_KEY = "MCPX_PLUGIN_INSTRUCTIONS";
const savedEnv = process.env[ENV_KEY];
afterEach(() => {
  if (savedEnv === undefined) delete process.env[ENV_KEY];
  else process.env[ENV_KEY] = savedEnv;
});

const stubStorage = (): StorageLike => {
  const map = new Map<string, unknown>();
  return {
    get: async (k) => map.get(k),
    set: async (k, v) => void map.set(k, v),
    remove: async (k) => void map.delete(k),
  };
};

type ContextInput = {
  sessionID: string;
  system: unknown[];
  tools: Record<string, unknown>;
};

// Setup exactly as the host performs it (v2/index.ts): the stub
// context captures the registered "context" hook so the test can
// fire it with the plain input objects the host would build.
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
  const fire = (input: ContextInput): ContextInput => {
    contextFn!(input as never);
    return input;
  };
  const fireNew = (sessionID: string): ContextInput => fire({ sessionID, system: [], tools: {} });
  return { ctx, fire, fireNew };
};

const setupWith = async (env: string | undefined, rawOptions?: Options) => {
  if (env === undefined) delete process.env[ENV_KEY];
  else process.env[ENV_KEY] = env;
  const { ctx, fire, fireNew } = stubCtx(stubStorage());
  await setupMcpxV2(ctx, rawOptions);
  return { fire, fireNew };
};

const texts = (input: ContextInput): string[] =>
  input.system.map((part) => (typeof part === "string" ? part : ((part as { text?: string }).text ?? "")));

describe("plugin.instructions (MCPX_PLUGIN_INSTRUCTIONS)", () => {
  test("unset: the context hook appends nothing", async () => {
    const { fireNew } = await setupWith(undefined);
    const input = fireNew("s1");
    expect(input.system).toEqual([]);
  });

  test("true: the paragraph is appended, once per system array", async () => {
    const { fire, fireNew } = await setupWith("true");
    const input = fireNew("s1");
    expect(texts(input)).toEqual([CONTEXT_PARAGRAPH]);
    expect(texts(input)[0]).toContain(CONTEXT_MARKER);
    // The hook fires per turn over the same system array; the
    // marker check keeps the append to exactly one paragraph.
    fire(input);
    expect(texts(input)).toEqual([CONTEXT_PARAGRAPH]);
    // A different session's fresh array gets its own copy.
    expect(texts(fireNew("s2"))).toEqual([CONTEXT_PARAGRAPH]);
  });

  test("explicit false: nothing is appended, exactly like unset", async () => {
    const { fireNew } = await setupWith("false");
    expect(fireNew("s1").system).toEqual([]);
  });

  test("a different truthy value appends the paragraph — the setting alone decides", async () => {
    const { fireNew } = await setupWith("1");
    expect(texts(fireNew("s1"))).toEqual([CONTEXT_PARAGRAPH]);
  });

  test("the host option beats the environment in both directions", async () => {
    const on = await setupWith(undefined, { instructions: true });
    expect(texts(on.fireNew("s1"))).toEqual([CONTEXT_PARAGRAPH]);
    const off = await setupWith("true", { instructions: false });
    expect(off.fireNew("s1").system).toEqual([]);
  });
});
