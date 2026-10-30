/**
 * mcpx for opencode v2 — the { id, setup } module (design doc §1).
 *
 * First slice: setup wires the pieces that need no tool surface yet —
 * the remembered-daemon read (ctx.storage), the idempotent system-prompt
 * paragraph (session context hook), the once-per-session full-environment
 * send, and execute.after timing records. Tool registration
 * (tool.transform, tools.ts in the design map) is the next slice; the v1
 * tree keeps serving tools until it lands.
 */
import { appendContextOnce, type ContextHookInput } from "./context.ts";
import { resolveOptions, type Options } from "./options.ts";
import { readRememberedDaemon, type StorageLike } from "./remember.ts";
import { createSessionEnv } from "./session-env.ts";
import { createTimingLog, type ExecuteAfterInput } from "./timing.ts";

/** The slice of the v2 setup Context this slice uses (promise/plugin.ts). */
export type McpxV2Context = {
  storage: StorageLike;
  serverUrl?: string;
  directory?: string;
  session: {
    hook(name: "context", fn: (input: ContextHookInput) => void): void;
  };
  tool: {
    hook(name: "execute.after", fn: (input: ExecuteAfterInput) => void): void;
  };
};

export function setupMcpxV2(ctx: McpxV2Context, rawOptions?: Options): void {
  const options = resolveOptions(rawOptions, process.env);
  const directory = ctx.directory ?? process.cwd();
  const sessionEnv = createSessionEnv({
    serverUrl: ctx.serverUrl,
    baseEnv: process.env,
  });
  const timing = createTimingLog();
  const toolTiming = rawOptions?.toolTiming ?? true;
  // Resolved now so later slices read settings from one place; the first
  // slice only needs toolTiming and the remember warm read.
  void options;

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
}

export default {
  id: "mcpx",
  setup: (ctx: McpxV2Context) => {
    setupMcpxV2(ctx);
  },
};
