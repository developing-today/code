/**
 * mcpx for opencode v2 — the { id, setup } module (design doc §1).
 *
 * Setup wires the whole core: the remembered-daemon read (ctx.storage),
 * the idempotent system-prompt paragraph (session context hook), the
 * per-session filtering of disabled mcpx tools from the model-visible
 * set (tool-filter.ts, #5122), the once-per-session full-environment
 * send, execute.after timing records,
 * the permission gate (permission.ts: evaluate hook + daemon_select
 * assert), tool registration (tools.ts: tool.transform under the mcpx
 * namespace, so the model sees the v1 names — mcpx_exec and friends),
 * and the sampling watcher (sampling.ts, #1858 item (4)): broker
 * sample rows answered through ctx.generate.text. The daemon picker
 * (picker.ts, #1858 item (3)) rides the tool surface: mcpx_daemon_pick
 * renders the candidate menu and confirms the choice through the
 * permission domain — the closest equivalent of a TUI select the
 * pinned Context supports (picker.ts records the evidence).
 * The v1 tree stays beside this one, untouched, until v2 is what runs.
 */
import { targetKey, type Target } from "../mcpx/daemon.ts";
import { appendContextOnce, type ContextHookInput } from "./context.ts";
import { resolveOptions, type Options } from "./options.ts";
import { wirePermissionGate, type PermissionDomainLike } from "./permission.ts";
import { readRememberedDaemon, type StorageLike } from "./remember.ts";
import { startSamplingWatcher, type GenerateTextLike, type SamplingDaemonLike } from "./sampling.ts";
import { createSessionEnv } from "./session-env.ts";
import { createToolFilter, readDisabledToolsPolicy, setDisabledTools, type ToolFilterInput } from "./tool-filter.ts";
import { createTimingLog, type ExecuteAfterInput, type ToolTimingRecord } from "./timing.ts";
import { createToolRuntime, registerTools, type ToolDomainLike, type ToolRuntime } from "./tools.ts";

/** The slice of the v2 setup Context this core uses (promise/plugin.ts). */
export type McpxV2Context = {
  storage: StorageLike;
  options?: Options;
  serverUrl?: string;
  directory?: string;
  session: {
    hook(name: "context", fn: (input: ContextHookInput & ToolFilterInput) => void): void;
  };
  tool: ToolDomainLike & {
    hook(name: "execute.after", fn: (input: ExecuteAfterInput) => void): void;
  };
  /**
   * The v2 permission domain (permission.ts): setup registers the
   * evaluate hook and wires assert into daemon_select. Optional in the
   * type because stub hosts omit it; a host without it — or whose
   * evaluate-hook registration fails — gets a daemon_select that
   * refuses to apply unconfirmed choices.
   */
  permission?: PermissionDomainLike;
  /**
   * The v2 generate domain (sampling.ts): one-shot completions.
   * Optional in the type because stub hosts omit it; without it the
   * sampling watcher never starts and broker sample rows are left
   * for another answerer, exactly as before item (4).
   */
  generate?: {
    text: GenerateTextLike;
  };
};

export async function setupMcpxV2(ctx: McpxV2Context, rawOptions?: Options): Promise<(() => void) | undefined> {
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
  // The per-session mcpx tool filter (#5122): its snapshots refresh
  // asynchronously from ctx.storage, and the context hook below applies
  // the latest snapshot synchronously — the hook never waits on storage
  // and never queries the daemon.
  const toolFilter = createToolFilter(ctx.storage);
  // The production writer for the filter's policy (#5122): the user's
  // disabledTools plugin option is recorded as a session's policy in
  // ctx.storage the first time the session is seen — unless the
  // session already has a policy of its own, which always wins. The
  // write is async (the hook never waits) and the refresh that follows
  // it puts the seeded policy in force from the session's next turn.
  const seedSessionPolicy = async (sessionID: string): Promise<void> => {
    // Seed only when the session has NO policy of its own: the
    // presence-aware read returns undefined for absence, while an
    // explicitly empty policy ({ disabled: [] } — the session's
    // opt-out of global filtering) returns an empty set and wins,
    // exactly like a nonempty one (#5122 Codex re-review, PR #5170).
    // A failed read (null here) also leaves the session untouched:
    // absence was not proven.
    const existing = await readDisabledToolsPolicy(ctx.storage, sessionID).catch(() => null);
    if (existing === undefined) {
      await setDisabledTools(ctx.storage, sessionID, options.disabledTools ?? []).catch(() => undefined);
    }
    await toolFilter.refresh(sessionID).catch(() => undefined);
  };
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
    const firstSighting = !envSessions.has(input.sessionID);
    envSessions.add(input.sessionID);
    void sessionEnv.ensure(input.sessionID, sessionVars(input.sessionID)).catch(() => false);
    // Hide this session's disabled mcpx tools from the model-visible
    // set (tool-filter.ts), then refresh the snapshot for the next
    // turn: a policy recorded since the last firing applies from the
    // following turn. On first sighting, seed the session's policy
    // from the disabledTools option first (seedSessionPolicy above
    // refreshes as its last step, so no second refresh is owed).
    toolFilter.apply(input);
    if (firstSighting && options.disabledTools && options.disabledTools.length > 0) {
      void seedSessionPolicy(input.sessionID);
    } else {
      void toolFilter.refresh(input.sessionID).catch(() => undefined);
    }
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
  // awaited and fail-closed (wirePermissionGate in permission.ts):
  // the assert callback exists only once the hook is registered, so
  // a failed registration leaves daemon_select refusing, never
  // silently applying.
  const assertPermission = await wirePermissionGate(ctx.permission);

  runtime = createToolRuntime({
    directory,
    env: process.env,
    options,
    storage: ctx.storage,
    onTarget: noteTarget,
    assertPermission,
  });
  await registerTools(ctx.tool, runtime, {
    toolsEnabled: options.tools ?? false,
    daemonTools: options.daemonTools,
  }).catch(() => []);

  // Sampling (#1858 item (4)): when the host offers ctx.generate.text
  // and the option has not turned it off, watch the daemon's broker
  // for sample rows routed to the agent and answer them. The watcher
  // resolves the daemon through the tool runtime on every poll, so
  // it follows daemon_select like everything else; a client without
  // the elicit methods (a stub, an older daemon core) reads as no
  // daemon. Setup returns the watcher's stop as its cleanup.
  const generate = ctx.generate;
  if (options.answerSampling === false || !generate) return undefined;
  const watcher = startSamplingWatcher({
    generate: generate.text,
    client: async (): Promise<SamplingDaemonLike | undefined> => {
      const client = (await runtime?.daemon().catch(() => undefined)) as Partial<SamplingDaemonLike> | undefined;
      if (typeof client?.pendingElicitations !== "function" || typeof client?.answer !== "function") {
        return undefined;
      }
      return client as SamplingDaemonLike;
    },
  });
  return () => watcher.stop();
}

export default {
  id: "mcpx",
  setup: (ctx: McpxV2Context) => setupMcpxV2(ctx, ctx.options),
};
