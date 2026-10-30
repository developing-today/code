/**
 * Plugin options for the v2 tree: the v1 Options type ported from
 * mcpx-session.ts, resolved the v1 way.
 *
 * Settings guard: TestThePluginReadsNoUndeclaredSetting scans plugin/**,
 * so the v2 tree reads only names the registry or the script-env contract
 * declares — the MCPX_PLUGIN_* names in internal/settings/plugin.go, plus
 * MCPX_DAEMON_ENDPOINT / MCPX_ENDPOINT / MCPX_SOCKET from scriptenv.go,
 * the same names v1 reads. The reads are
 * property accesses (env.MCPX_PLUGIN_BIN and friends), never quoted
 * literals: TestTheScriptEnvironmentIsWhatTheCodeSets treats a quoted
 * MCPX_* name as a variable the plugin SETS into a child's environment,
 * and these are reads, not sets. (An earlier draft listed the names in
 * a quoted array and failed that test on all twelve.)
 */
import { goDurationMs, splitArgs } from "../mcpx/daemon.ts";

export type Options = {
  /** A daemon named outright. `unix:///path` or `http://host:port`. */
  endpoint?: string;
  /** A socket named outright. */
  socket?: string;
  /** The mcpx binary, or "" to forbid spawning one. */
  bin?: string;
  /** Arguments to put before mcpx's own, as a vector or a string. */
  binArgs?: string[] | string;
  /** Where the opt-in tools get their answers: auto, v1 or cli. */
  backend?: "auto" | "v1" | "cli";
  /** Default scope for a daemon choice made this session. */
  remember?: "session" | "until-gone" | "indefinite";
  /** How much to inject into shell environments. */
  env?: "minimal" | "standard" | "full";
  instructions?: boolean;
  toolTiming?: boolean;
  tools?: boolean;
  /** Offer mcpx_daemon_status and mcpx_daemon_select. Default: when unclear. */
  daemonTools?: boolean;
  /** Report ambiguity on tool results. Default on. */
  annotate?: boolean;
  /** Override the "is anyone looking at this" guess. */
  headless?: boolean;
};

const str = (v: unknown): string | undefined =>
  v === undefined || v === null || v === "" ? undefined : String(v);

/**
 * The bin resolution only: nullish, the v1 way. An explicit "" is a
 * decision — forbid spawning — not an absence: str() would collapse it
 * to undefined and runBin would fall back to MCPX_PLUGIN_BIN or the
 * default binary, spawning exactly what the user forbade.
 */
const binStr = (v: unknown): string | undefined =>
  v === undefined || v === null ? undefined : String(v);

/** The v1 truthy parse: an unrecognised value is undefined, not false. */
const truthy = (v: unknown): boolean | undefined => {
  if (v === undefined || v === null || v === "") return undefined;
  if (typeof v === "boolean") return v;
  const s = String(v).toLowerCase();
  if (s === "1" || s === "true" || s === "yes" || s === "on") return true;
  if (s === "0" || s === "false" || s === "no" || s === "off") return false;
  return undefined;
};

/**
 * Resolve raw options the v1 way — plugin option first, then the
 * environment variable, then the caller's default — for the settings
 * the v2 core needs. Unset fields stay undefined, except toolTiming,
 * whose v1 default is false (opt-in): a resolved false is a decision,
 * and index.ts registers no timing hook for it.
 *
 * `discoveryRetryMs` is not a v1 Options field; it is the resolved
 * plugin.discoveryRetry setting (a duration the registry declares and
 * v1 applied to its discovery ladder), carried beside the options
 * because tools.ts's ladder is its only consumer in this tree.
 *
 * Still unresolved here, deliberately: env / headless. The v2 core
 * has no consumer for them yet — env sized v1's shell.env payload
 * (the v2 environment route sends the full env by design, §3), and
 * headless gated v1's toasts (the v2 context slice has no TUI
 * channel). Resolving a setting nothing reads would be the
 * declared-but-unread bug in a new costume. (instructions was on
 * this list until the tools slice: the context hook now gates its
 * paragraph on it, exactly as v1's system transform did, so it is
 * resolved above.)
 */
export type ResolvedOptions = Options & { discoveryRetryMs?: number };

export function resolveOptions(
  raw: Options | undefined,
  env: Record<string, string | undefined>,
): ResolvedOptions {
  const o = raw ?? {};
  return {
    endpoint: str(o.endpoint) ?? str(env.MCPX_DAEMON_ENDPOINT) ?? str(env.MCPX_ENDPOINT),
    socket: str(o.socket) ?? str(env.MCPX_SOCKET),
    bin: binStr(o.bin) ?? str(env.MCPX_PLUGIN_BIN),
    binArgs: Array.isArray(o.binArgs)
      ? o.binArgs
      : typeof o.binArgs === "string"
        ? splitArgs(o.binArgs)
        : splitArgs(env.MCPX_PLUGIN_BIN_ARGS),
    backend: (str(o.backend) ??
      str(env.MCPX_PLUGIN_BACKEND) ??
      "auto") as Options["backend"],
    remember: (str(o.remember) ??
      str(env.MCPX_PLUGIN_REMEMBER) ??
      "session") as Options["remember"],
    env: (str(o.env) ?? str(env.MCPX_PLUGIN_ENV)) as Options["env"],
    tools: o.tools ?? truthy(env.MCPX_PLUGIN_TOOLS) ?? false,
    instructions: o.instructions ?? truthy(env.MCPX_PLUGIN_INSTRUCTIONS) ?? false,
    daemonTools: o.daemonTools ?? truthy(env.MCPX_PLUGIN_DAEMON_TOOLS),
    annotate: o.annotate ?? truthy(env.MCPX_PLUGIN_ANNOTATE) ?? true,
    toolTiming: o.toolTiming ?? truthy(env.MCPX_PLUGIN_TOOL_TIMING) ?? false,
    discoveryRetryMs: goDurationMs(env.MCPX_PLUGIN_DISCOVERY_RETRY),
  };
}
