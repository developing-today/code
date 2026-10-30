/**
 * mcpx for opencode v2 — the { id, setup } module (design doc §1).
 *
 * Setup wires the whole core: the remembered-daemon read (ctx.storage),
 * the idempotent system-prompt paragraph (session context hook), the
 * once-per-session full-environment send, execute.after timing records,
 * and tool registration (tools.ts: tool.transform under the mcpx
 * namespace, so the model sees the v1 names — mcpx_exec and friends).
 * The v1 tree stays beside this one, untouched, until v2 is what runs.
 */
import { appendContextOnce, type ContextHookInput } from "./context.ts";
import { resolveOptions, type Options } from "./options.ts";
import { readRememberedDaemon, type StorageLike } from "./remember.ts";
import { createSessionEnv } from "./session-env.ts";
import { createTimingLog, type ExecuteAfterInput } from "./timing.ts";
import {
  createToolRuntime,
  registerTools,
  type ToolDomainLike,
} from "./tools.ts";

/** The slice of the v2 setup Context this core uses (promise/plugin.ts). */
export type McpxV2Context = {
  storage: StorageLike;
  options?: Options;
  serverUrl?: string;
  directory?: string;
  session: {
    hook(name: "context", fn: (input: ContextHookInput) => void): void;
  };
  tool: ToolDomainLike & {
    hook(name: "execute.after", fn: (input: ExecuteAfterInput) => void): void;
  };
};

export async function setupMcpxV2(ctx: McpxV2Context, rawOptions?: Options): Promise<void> {
  const options = resolveOptions(rawOptions, process.env);
  const directory = ctx.directory ?? process.cwd();
  const sessionEnv = createSessionEnv({
    serverUrl: ctx.serverUrl,
    baseEnv: process.env,
  });
  const timing = createTimingLog();
  // toolTiming comes from the resolved options (host option, then the
  // environment, then the v1 default false) — never a bare ?? true: a
  // user who turned timing off gets no hook, and no records array
  // growing for the life of the plugin instance.
  const toolTiming = options.toolTiming ?? false;

  // Best-effort warm read: a remembered daemon for this directory means the
  // first discovery in the session can skip the ladder's cold rungs. The
  // storage read must never fail setup.
  void readRememberedDaemon(ctx.storage, directory).catch(() => undefined);

  ctx.session.hook("context", (input) => {
    appendContextOnce(input);
    // First sighting of a session is when its environment is sent — once,
    // with the full environment (session-env.ts explains why full).
    void sessionEnv
      .ensure(input.sessionID, {
        MCPX_SESSION_ID: input.sessionID,
        MCPX_DIRECTORY: directory,
      })
      .catch(() => false);
  });

  if (toolTiming) {
    ctx.tool.hook("execute.after", (input) => {
      timing.hook(input);
    });
  }

  // Tools last, and never allowed to fail setup: a plugin that throws
  // during load loses the session bridge and the environment send too,
  // which are worth more than the tool surface. Registration does its
  // own boot discovery for the daemon-tools default (tools.ts).
  const runtime = createToolRuntime({
    directory,
    env: process.env,
    options,
    storage: ctx.storage,
  });
  await registerTools(ctx.tool, runtime, {
    toolsEnabled: options.tools ?? false,
    daemonTools: options.daemonTools,
  }).catch(() => []);
}

export default {
  id: "mcpx",
  setup: (ctx: McpxV2Context) => setupMcpxV2(ctx, ctx.options),
};
