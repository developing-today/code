/**
 * Plugin options for the v2 tree: the v1 `Options` type ported verbatim from
 * mcpx-session.ts, plus the declared setting variable names.
 *
 * Settings guard: TestThePluginReadsNoUndeclaredSetting scans plugin/**, so
 * the v2 tree may read only the MCPX_PLUGIN_* names declared here (the same
 * names v1 reads, which are the names in internal/settings/plugin.go).
 */
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

/** Every environment variable the plugin may read (settings-guard list). */
export const PLUGIN_SETTING_VARS = [
  "MCPX_PLUGIN_ANNOTATE",
  "MCPX_PLUGIN_BACKEND",
  "MCPX_PLUGIN_BIN",
  "MCPX_PLUGIN_BIN_ARGS",
  "MCPX_PLUGIN_DAEMON_TOOLS",
  "MCPX_PLUGIN_DISCOVERY_RETRY",
  "MCPX_PLUGIN_ENV",
  "MCPX_PLUGIN_HEADLESS",
  "MCPX_PLUGIN_INSTRUCTIONS",
  "MCPX_PLUGIN_REMEMBER",
  "MCPX_PLUGIN_TOOLS",
  "MCPX_PLUGIN_TOOL_TIMING",
] as const;

const str = (v: unknown): string | undefined =>
  v === undefined || v === null || v === "" ? undefined : String(v);

/**
 * Resolve raw options the v1 way — plugin option first, then the environment
 * variable, then the caller's default — for the string-valued settings the
 * v2 core needs at setup time. Unset fields stay undefined.
 */
export function resolveOptions(
  raw: Options | undefined,
  env: Record<string, string | undefined>,
): Options {
  const o = raw ?? {};
  const pick = (
    option: unknown,
    varName: (typeof PLUGIN_SETTING_VARS)[number],
    fallback?: string,
  ): string | undefined => str(option) ?? str(env[varName]) ?? fallback;
  return {
    endpoint: o.endpoint,
    socket: o.socket,
    bin: pick(o.bin, "MCPX_PLUGIN_BIN"),
    backend: (pick(o.backend, "MCPX_PLUGIN_BACKEND", "auto") ?? "auto") as Options["backend"],
    remember: (pick(o.remember, "MCPX_PLUGIN_REMEMBER", "session") ??
      "session") as Options["remember"],
    env: (pick(o.env, "MCPX_PLUGIN_ENV") ?? undefined) as Options["env"],
  };
}
