/**
 * mcpx for opencode v2 — the { id, setup } module (design doc §1).
 *
 * Setup wires the whole core: the remembered-daemon read (ctx.storage),
 * the idempotent system-prompt paragraph (session context hook), the
 * once-per-session full-environment send, execute.after timing records,
 * the permission gate (permission.ts: evaluate hook + daemon_select
 * assert), and tool registration (tools.ts: tool.transform under the mcpx
 * namespace, so the model sees the v1 names — mcpx_exec and friends).
 * The v1 tree stays beside this one, untouched, until v2 is what runs.
 */
import { targetKey, type Target } from "../mcpx/daemon.ts";
import { appendContextOnce, type ContextHookInput } from "./context.ts";
import { resolveOptions, type Options } from "./options.ts";
import {
  evaluateMcpxPermission,
  type PermissionDomainLike,
} from "./permission.ts";
import { readRememberedDaemon, type StorageLike } from "./remember.ts";
import { createSessionEnv } from "./session-env.ts";
import {
  createTimingLog,
  type ExecuteAfterInput,
  type ToolTimingRecord,
} from "./timing.ts";
import {
  createToolRuntime,
  registerTools,
  type ToolDomainLike,
  type ToolRuntime,
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
  /**
   * The v2 permission domain (permission.ts): setup registers the
   * evaluate hook and wires assert into daemon_select. Optional in the
   * type because stub hosts omit it; a host without it gets a
   * daemon_select that refuses to apply unconfirmed choices.
   */
  permission?: PermissionDomainLike;
};

export async function setupMcpxV2(ctx: McpxV2Context, rawOptions?: Options): Promise<void> {
  const options = resolveOptions(rawOptions, process.env);
  const directory = ctx.directory ?? process.cwd();
  const sessionEnv = createSessionEnv({
    serverUrl: ctx.serverUrl,
    baseEnv: process.env,
  });
  // The daemon endpoint rides in the session environment, as v1's
  // shell.env hook injected it: a shell `mcpx` command must talk to the
  // same daemon the plugin tools use, not walk its own ladder to a
  // possibly different one. MCPX_DAEMON_ENDPOINT, not MCPX_SOCKET —
  // the endpoint is only read when connecting, while the socket also
  // decides where a newly started daemon binds (v1's reasoning,
  // mcpx-session.ts). Sessions are tracked so a daemon_select later in
  // the session can refresh the ones already sent.
  const envSessions = new Set<string>();
  let activeEndpoint: string | undefined;
  const sessionVars = (sessionID: string): Record<string, string> => {
    const vars: Record<string, string> = {
      MCPX_SESSION_ID: sessionID,
      MCPX_DIRECTORY: directory,
    };
    if (activeEndpoint !== undefined) vars.MCPX_DAEMON_ENDPOINT = activeEndpoint;
    return vars;
  };
  const noteTarget = (target: Target): void => {
    const endpoint = targetKey(target);
    if (endpoint === activeEndpoint) return;
    activeEndpoint = endpoint;
    for (const sessionID of envSessions) {
      void sessionEnv.update(sessionID, sessionVars(sessionID)).catch(() => false);
    }
  };
  // The runtime is created before the timing log: the log's sink
  // delivers through it (below), and the closure only runs once a tool
  // has executed — long after both exist.
  let runtime: ToolRuntime | undefined;
  // toolTiming's sink is v1's path: the record goes to the daemon's
  // durable log (`daemon.record`, event "harness.tool"), so one
  // `mcpx stats` / mcpx_observe covers harness tool calls too. Best
  // effort exactly as v1: with no daemon the record is dropped rather
  // than spawning a process per tool call, and a failed send never
  // fails the tool call it describes.
  const deliverTiming = (record: ToolTimingRecord): void => {
    void (async () => {
      const client = await runtime?.daemon().catch(() => undefined);
      if (!client) return;
      await client
        .record({
          event: "harness.tool",
          tool: record.tool,
          session: record.sessionID,
          status: record.status,
        })
        .catch(() => undefined);
    })();
  };
  const timing = createTimingLog(deliverTiming);
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
    // The paragraph is gated on plugin.instructions, off by default,
    // exactly as v1's system transform was: prompt content is paid on
    // every request, so it is opt-in help. The environment send below
    // is NOT gated — session identity is plumbing, not help.
    if (options.instructions) appendContextOnce(input);
    // First sighting of a session is when its environment is sent — once,
    // with the full environment (session-env.ts explains why full).
    envSessions.add(input.sessionID);
    void sessionEnv.ensure(input.sessionID, sessionVars(input.sessionID)).catch(() => false);
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
  // The permission gate (#1858 item (2)): the evaluate hook keeps
  // mcpx's confirmation action at "ask", and daemon_select asserts
  // through the same domain before applying a choice. Registration is
  // best-effort like everything else in setup — a failed hook leaves
  // daemon_select refusing, never silently applying.
  if (ctx.permission) {
    void ctx.permission.hook("evaluate", evaluateMcpxPermission).catch(() => undefined);
  }

  runtime = createToolRuntime({
    directory,
    env: process.env,
    options,
    storage: ctx.storage,
    onTarget: noteTarget,
    assertPermission: ctx.permission
      ? (input) => ctx.permission!.assert(input)
      : undefined,
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
