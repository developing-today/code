/**
 * Tool registration for the v2 tree: ctx.tool.transform + a namespace
 * (design doc §6, and §7's namespace question, resolved against the pin).
 *
 * Verified against the pinned v2 source (packages/plugin/src/promise/tool.ts
 * and packages/core/src/tool/runtime.ts):
 *
 *  - The editor is `namespace({ name, description })` then `add(tool)` —
 *    namespace takes an object, not a bare string.
 *  - A tool is `{ name, description, input, execute }` where `input` is a
 *    ValueSchema: an effect Schema codec, a StandardSchema, or a plain
 *    JSON Schema object. This tree uses plain JSON Schema objects, so the
 *    plugin needs no host imports — the same structural-typing approach as
 *    the rest of the v2 tree.
 *  - The model-facing name is `<namespace>_<name>` (runtime.ts
 *    effectiveName), so registering name `exec` under namespace `mcpx`
 *    yields `mcpx_exec` — exactly the names the v1 tools were given by
 *    hand. Nothing downstream (prompts, docs, muscle memory) changes.
 *  - execute returns a Tool.Result; `{ content: string }` is the shape
 *    that carries the v1 tools' string bodies across unchanged.
 *
 * What differs from v1, honestly:
 *
 *  - No ctx.ask. v1's daemon_select confirmed through the tool context's
 *    permission prompt; the v2 ToolContext has no ask, and the permission
 *    domain (permission.hook("evaluate")) is SET-REMAINDER work on #1858,
 *    not this core. So v2 daemon_select applies the choice it was told to
 *    apply. Its description no longer claims a confirmation happens.
 *  - No toasts. The v2 server context slice this tree uses has no TUI
 *    channel; ambiguity is surfaced the other v1 way — the annotation line
 *    on tool results — which needs no interface at all.
 *  - The remembered daemon lives in ctx.storage (remember.ts), not the
 *    v1 remember file. The TUI picker channel that justified re-reading
 *    the file every few seconds does not exist in v2's two-process model;
 *    the storage record is still re-read on the same cadence, cheaply, so
 *    a choice made by another surface in the same host is picked up.
 */
import {
  DaemonClient,
  TUNING,
  ago,
  discover,
  label,
  rank,
  runBin,
  splitArgs,
  targetKey,
  type Candidate,
  type Discovery,
  type DiscoverOptions,
  type Target,
} from "../mcpx/daemon.ts";
import type { Options } from "./options.ts";
import {
  forgetDaemon,
  readRememberedDaemon,
  rememberDaemon,
  type StorageLike,
} from "./remember.ts";

/** The slice of the v2 ToolEditor this module uses (promise/tool.ts). */
export type ToolEditorLike = {
  namespace(ns: { name: string; description: string }): void;
  add(tool: McpxToolInfo): void;
};

/** The slice of ctx.tool this module uses: transform, nothing else. */
export type ToolDomainLike = {
  transform(fn: (editor: ToolEditorLike) => void): Promise<unknown>;
};

/** A tool definition in the v2 shape, with a JSON Schema input. */
export type McpxToolInfo = {
  name: string;
  description: string;
  input: Record<string, unknown>;
  options?: { namespace?: string };
  execute: (
    input: never,
    context: { sessionID: string },
  ) => Promise<{ content: string }>;
};

/** The slice of DaemonClient the tools use (structural, for tests). */
export type DaemonClientLike = {
  namespaces(): Promise<Array<{ namespace: string; tools: number; description?: string }>>;
  types(ns?: string): Promise<string>;
  exec(source: string): Promise<{ output?: string; result?: unknown } | undefined>;
  logQuery(filter: { limit: number; since?: string }): Promise<{ records: unknown }>;
  stats(opts: { by: string; since?: string }): Promise<unknown>;
  putSetting(key: string, value: unknown): Promise<"ok" | "absent" | "failed">;
  shutdown(): Promise<boolean>;
  /** Append a record to the daemon's durable log (DaemonClient.record). */
  record(record: Record<string, unknown>): Promise<unknown>;
};

export const NAMESPACE = "mcpx";

export const NAMESPACE_DESCRIPTION =
  "mcpx leases MCP servers per session through a local daemon and drives " +
  "them all at once: discover what is available, exec TypeScript against " +
  "every server in one call, and observe what the daemon has been doing.";

const EMPTY_INPUT = { type: "object", properties: {}, additionalProperties: false } as const;

export type ToolRuntimeDeps = {
  directory: string;
  env: Record<string, string | undefined>;
  options: Options & { discoveryRetryMs?: number };
  storage: StorageLike;
  discoverFn?: typeof discover;
  runBinFn?: typeof runBin;
  clientFor?: (target: Target) => DaemonClientLike;
  /**
   * The active daemon target changed (discovery settled on one, or a
   * daemon_select applied a new choice). index.ts uses it to keep the
   * session environment's MCPX_DAEMON_ENDPOINT pointing at the daemon
   * the tools are actually using — v1 injected it from its shell.env
   * hook for exactly this reason.
   */
  onTarget?: (target: Target) => void;
};

export type ToolRuntime = {
  look(): Promise<Discovery>;
  daemon(): Promise<DaemonClientLike | undefined>;
  tools(): McpxToolInfo[];
  daemonTools(): McpxToolInfo[];
  daemonToolsWanted(boot: Discovery): boolean;
};

/**
 * The daemon-facing runtime the tools share: the v1 ladder, caching and
 * fallback logic from mcpx-session.ts, with ctx.storage standing in for
 * the remember file. One runtime per plugin instance (per setup call).
 */
export function createToolRuntime(deps: ToolRuntimeDeps): ToolRuntime {
  const options = deps.options;
  const discoverFn = deps.discoverFn ?? discover;
  const runBinFn = deps.runBinFn ?? runBin;
  const clientFor =
    deps.clientFor ?? ((target: Target) => new DaemonClient(target) as DaemonClientLike);
  const discoveryRetryMs = options.discoveryRetryMs ?? TUNING.missCooldownMs;

  const ladderOptions = (remembered?: Target): DiscoverOptions => ({
    directory: deps.directory,
    endpoint: options.endpoint,
    socket: options.socket,
    bin: options.bin,
    binArgs: Array.isArray(options.binArgs)
      ? options.binArgs
      : splitArgs(options.binArgs),
    remembered: remembered ?? chosen,
    // The v2 ladder is storage-backed (remember.ts). Without this the
    // shared discover() falls back to the v1 remember FILE whenever no
    // target is supplied — and daemon_forget, which removes only the
    // storage record, would instantly rediscover the daemon it just
    // forgot from the legacy file.
    useRememberFile: false,
    env: deps.env,
  });

  // The daemon, found once and reused. A miss is remembered for a while
  // rather than retried on every tool call (v1, unchanged reasoning).
  let chosen: Target | undefined;
  let found: Promise<Discovery> | undefined;
  let missedAt = 0;
  // The last target reported through onTarget, so a stable daemon is
  // reported once, not on every look.
  let knownTargetKey: string | undefined;

  // The remembered choice, re-read on v1's cadence: another surface in
  // the same host may change it, and never re-reading would make that
  // take effect only next session; always re-reading puts a storage
  // round-trip in front of every tool call.
  let rememberedKey: string | undefined;
  let rememberedAt = 0;
  const refreshRemembered = async (): Promise<boolean> => {
    if (Date.now() - rememberedAt < TUNING.rememberRecheckMs) return false;
    rememberedAt = Date.now();
    const record = await readRememberedDaemon(deps.storage, deps.directory).catch(
      () => undefined,
    );
    const now = record ? targetKey(record.target) : undefined;
    if (now === rememberedKey) return false;
    rememberedKey = now;
    return true;
  };

  const look = async (): Promise<Discovery> => {
    if (found && (await refreshRemembered())) {
      found = undefined;
      chosen = undefined;
    }
    if (!found) {
      if (Date.now() - missedAt < discoveryRetryMs && missedAt !== 0) {
        return { candidates: [], rung: 6, reason: "none", ambiguous: false };
      }
      await refreshRemembered();
      const record = await readRememberedDaemon(deps.storage, deps.directory).catch(
        () => undefined,
      );
      // A choice made this session outranks the stored one: after a
      // transient failure clears `found`, rediscovery must not silently
      // switch back to the stored daemon a daemon_select moved away
      // from. (ladderOptions would take the passed target over chosen,
      // so the precedence is decided here, chosen first — as v1 had it.)
      found = discoverFn(ladderOptions(chosen ?? record?.target));
    }
    const d = await found;
    if (!d.target) {
      found = undefined;
      missedAt = Date.now();
    } else if (knownTargetKey !== targetKey(d.target)) {
      knownTargetKey = targetKey(d.target);
      deps.onTarget?.(d.target);
    }
    return d;
  };

  const daemon = async (): Promise<DaemonClientLike | undefined> => {
    const d = await look();
    return d.target ? clientFor(d.target) : undefined;
  };

  const forget = (): void => {
    found = undefined;
    missedAt = Date.now();
  };

  /**
   * Run something against the daemon, falling back to the binary.
   * `backend` decides how hard to try: `v1` never spawns, `cli` never
   * uses the socket, `auto` prefers the socket (v1, unchanged).
   */
  const viaDaemon = async <T>(
    fn: (c: DaemonClientLike) => Promise<T>,
  ): Promise<T | undefined> => {
    if (options.backend === "cli") return undefined;
    const c = await daemon();
    if (!c) return undefined;
    try {
      return await fn(c);
    } catch (err) {
      // Stopped, restarted, or its socket moved. Forget it; the next
      // call asks again.
      forget();
      throw err;
    }
  };

  const viaCLI = async (argv: string[]): Promise<string> => {
    if (options.backend === "v1") {
      return "mcpx: no daemon is reachable, and backend=v1 forbids running the binary.";
    }
    const out = await runBinFn(ladderOptions(), argv);
    if (out === undefined) {
      return (
        "mcpx: no daemon is reachable and no mcpx binary could be run. " +
        "Start one with `mcpx daemon`, or point the plugin at one with MCPX_DAEMON_ENDPOINT."
      );
    }
    return out || "(no output)";
  };

  const pretty = (v: unknown): string => JSON.stringify(v, null, 2);

  /** One line on every mcpx result, naming the daemon that answered (v1). */
  const annotation = async (): Promise<string> => {
    const d = await look();
    if (options.annotate === false || !d.ambiguous || !d.chosen) return "";
    return (
      `\n\n---\nmcpx: answered by ${label(d.chosen)} (${targetKey(d.chosen.target)}); ` +
      `${d.candidates.length} daemons match this directory. ` +
      `Call mcpx_daemon_status to see them, mcpx_daemon_select to change.`
    );
  };

  /** One candidate as a line in the status table (v1, unchanged). */
  const describeCandidate = (c: Candidate, i: number, current?: Target): string => {
    const mark = current && targetKey(current) === targetKey(c.target) ? "*" : " ";
    const bits = [
      `${mark} [${i}] ${label(c)}`,
      `      config:   ${c.configPath ?? "(unknown)"}`,
      `      socket:   ${targetKey(c.target)}`,
    ];
    if (c.startedAt) bits.push(`      started:  ${c.startedAt} (${ago(c.startedAt)})`);
    if (c.version) bits.push(`      version:  ${c.version}`);
    if (c.servers !== undefined) bits.push(`      servers:  ${c.servers}`);
    if (c.sessions !== undefined) bits.push(`      sessions: ${c.sessions}`);
    if (c.instances !== undefined) bits.push(`      live:     ${c.instances}`);
    return bits.join("\n");
  };

  /** Apply a daemon choice, at the scope asked for (v1 scopes, storage-backed). */
  const applyChoice = async (
    target: Target,
    scope: "session" | "until-gone" | "indefinite" | "permanent",
  ): Promise<string> => {
    chosen = target;
    if (knownTargetKey !== targetKey(target)) {
      knownTargetKey = targetKey(target);
      deps.onTarget?.(target);
    }
    found = Promise.resolve(await discoverFn({ ...ladderOptions(), remembered: target }));
    if (scope === "session") return "Remembered for this session only.";
    if (scope === "permanent") {
      const res = await clientFor(target).putSetting("daemon.endpoint", targetKey(target));
      if (res === "ok") return "Written to your mcpx configuration as daemon.endpoint.";
      if (res === "absent") {
        // Being added concurrently. Say what to do rather than failing,
        // and leave a durable choice behind (v1 did the same).
        await rememberDaemon(deps.storage, deps.directory, target);
        return (
          "This daemon has no settings API (PUT /v1/settings returned 404), so the choice " +
          "was remembered indefinitely for this directory instead. To make it permanent, " +
          `set "daemon": { "endpoint": "${targetKey(target)}" } in your mcpx config.`
        );
      }
      return "Writing the setting failed; the choice holds for this session.";
    }
    // until-gone and indefinite both persist to ctx.storage; until-gone's
    // extra lifetime rule (drop the record when the health check fails)
    // is recallDaemon's, applied on the setup warm read.
    await rememberDaemon(deps.storage, deps.directory, target);
    return scope === "until-gone"
      ? "Remembered until this daemon stops answering."
      : "Remembered indefinitely for this directory.";
  };

  const result = (text: string): { content: string } => ({ content: text });

  const daemonTools: McpxToolInfo[] = [
    {
      name: "daemon_status",
      description:
        "List every mcpx daemon running on this machine, best match first, and say " +
        "which one this session is using. Call this before mcpx_daemon_select: the " +
        "index it prints is what select takes.",
      input: { ...EMPTY_INPUT },
      options: { namespace: NAMESPACE },
      execute: async () => {
        const record = await readRememberedDaemon(deps.storage, deps.directory).catch(
          () => undefined,
        );
        const d = await discoverFn({
          ...ladderOptions(),
          remembered: chosen ?? record?.target,
        });
        const list = rank(d.candidates, deps.directory);
        if (list.length === 0) {
          if (d.target) {
            return result(`Using ${targetKey(d.target)} (named explicitly; no scan was done).`);
          }
          return result(
            "No mcpx daemon is running. Start one with `mcpx daemon`, or set MCPX_DAEMON_ENDPOINT.",
          );
        }
        const head =
          `${list.length} daemon${list.length === 1 ? "" : "s"} running. ` +
          (d.target ? `This session is using ${targetKey(d.target)} (${d.reason}).` : "") +
          "\n\n";
        return result(head + list.map((c, i) => describeCandidate(c, i, d.target)).join("\n\n"));
      },
    },
    {
      name: "daemon_select",
      description:
        "Point this session at one of the daemons mcpx_daemon_status listed, by its " +
        "index. The tool sets the socket itself — pass the index, never a path.",
      input: {
        type: "object",
        properties: {
          candidate: {
            type: "integer",
            description: "the [index] from mcpx_daemon_status",
          },
          remember: {
            type: "string",
            enum: ["session", "until-gone", "indefinite", "permanent"],
            description:
              "session: this session only (default). until-gone: until that daemon stops " +
              "answering. indefinite: for this directory, until changed. permanent: " +
              "written into the mcpx config.",
          },
          stopOthers: {
            type: "boolean",
            description: "stop every other daemon after selecting. Destructive: their sessions end.",
          },
        },
        required: ["candidate"],
        additionalProperties: false,
      },
      options: { namespace: NAMESPACE },
      execute: async (input: { candidate: number; remember?: string; stopOthers?: boolean }) => {
        const record = await readRememberedDaemon(deps.storage, deps.directory).catch(
          () => undefined,
        );
        const d = await discoverFn({
          ...ladderOptions(),
          remembered: chosen ?? record?.target,
        });
        const list = rank(d.candidates, deps.directory);
        if (list.length === 0) return result("No daemon to select. Run mcpx_daemon_status first.");
        const pick = list[input.candidate];
        if (!pick) {
          return result(
            `No candidate [${input.candidate}]. mcpx_daemon_status lists 0..${list.length - 1}.`,
          );
        }
        const scope = (input.remember ??
          options.remember ??
          "session") as "session" | "until-gone" | "indefinite" | "permanent";
        const note = await applyChoice(pick.target, scope);
        let stopped = "";
        if (input.stopOthers) {
          const others = list.filter((c) => targetKey(c.target) !== targetKey(pick.target));
          const results = await Promise.all(
            others.map(async (c) => ({ c, ok: await clientFor(c.target).shutdown() })),
          );
          stopped =
            "\nStopped: " +
            (results.length
              ? results.map((r) => `${label(r.c)}${r.ok ? "" : " (failed)"}`).join(", ")
              : "none");
        }
        return result(`Now using ${label(pick)} (${targetKey(pick.target)}). ${note}${stopped}`);
      },
    },
    {
      name: "daemon_forget",
      description:
        "Drop the remembered daemon choice for this directory, so discovery decides again.",
      input: { ...EMPTY_INPUT },
      options: { namespace: NAMESPACE },
      execute: async () => {
        await forgetDaemon(deps.storage, deps.directory);
        chosen = undefined;
        found = undefined;
        const d = await look();
        return result(
          d.target
            ? `Forgotten. Discovery now chooses ${targetKey(d.target)} (${d.reason}).`
            : "Forgotten. No daemon is reachable.",
        );
      },
    },
  ];

  const mcpxTools: McpxToolInfo[] = [
    {
      name: "discover",
      description:
        "List the MCP servers mcpx knows about, or show signatures for one. " +
        "Call with no arguments first: the answer is small and tells you " +
        "what else is worth asking for.",
      input: {
        type: "object",
        properties: {
          namespace: {
            type: "string",
            description: "a namespace to show signatures for; omit to list all",
          },
        },
        additionalProperties: false,
      },
      options: { namespace: NAMESPACE },
      execute: async (input: { namespace?: string }) => {
        const out = await viaDaemon(async (c) => {
          if (input.namespace) return await c.types(input.namespace);
          const ns = await c.namespaces();
          return ns
            .map(
              (n) => `${n.namespace}  (${n.tools} tools)${n.description ? "  " + n.description : ""}`,
            )
            .join("\n");
        });
        const body = out ?? (await viaCLI(input.namespace ? ["types", input.namespace] : ["ls"]));
        return result(body + (await annotation()));
      },
    },
    {
      name: "exec",
      description:
        "Run TypeScript against every MCP server at once. Tools are bound as " +
        "await tools.<namespace>.<tool>({...}) and only what you print comes " +
        "back, so filter and summarise here rather than reading a megabyte " +
        "of JSON into your context. This is the one that saves tokens.",
      input: {
        type: "object",
        properties: {
          source: { type: "string", description: "TypeScript; top-level await works" },
        },
        required: ["source"],
        additionalProperties: false,
      },
      options: { namespace: NAMESPACE },
      execute: async (input: { source: string }) => {
        // undefined from the daemon means the route is absent — an older
        // daemon — which is a reason to fall back, not a failure to report.
        const out = await viaDaemon((c) => c.exec(input.source));
        const body =
          out === undefined ? await viaCLI(["exec", input.source]) : (out.output ?? pretty(out.result ?? out));
        return result(body + (await annotation()));
      },
    },
    {
      name: "observe",
      description:
        "Query what mcpx has been doing: the durable log, or aggregate " +
        "statistics. Use it when a call failed and you want to know why " +
        "without running it again.",
      input: {
        type: "object",
        properties: {
          what: {
            type: "string",
            enum: ["log", "calls", "errors", "servers", "slowest"],
            description: "log for records, the rest are aggregates",
          },
          since: { type: "string", description: "15m, 2h, or an RFC3339 time" },
        },
        required: ["what"],
        additionalProperties: false,
      },
      options: { namespace: NAMESPACE },
      execute: async (input: { what: string; since?: string }) => {
        const out = await viaDaemon(async (c) => {
          if (input.what === "log") {
            const res = await c.logQuery({ limit: TUNING.observeLimit, since: input.since });
            return pretty(res.records);
          }
          return pretty(await c.stats({ by: input.what, since: input.since }));
        });
        if (out !== undefined) return result(out + (await annotation()));
        const argv =
          input.what === "log"
            ? ["log", "--limit", String(TUNING.observeLimit), ...(input.since ? ["--since", input.since] : [])]
            : ["stats", input.what, ...(input.since ? ["--since", input.since] : [])];
        return result((await viaCLI(argv)) + (await annotation()));
      },
    },
  ];

  return {
    look,
    daemon,
    tools: () => mcpxTools,
    daemonTools: () => daemonTools,
    daemonToolsWanted: (boot: Discovery): boolean =>
      // Offered when they can help: asked for outright, when the full
      // tool set is on, or when discovery found more than one daemon
      // and somebody may want to choose (v1's rule, tri-state intact).
      options.daemonTools ?? (options.tools === true || boot.ambiguous),
  };
}

/**
 * Register the namespace and the gated tool sets on the host.
 *
 * The daemon tools' default ("when unclear") needs a boot discovery —
 * v1 awaited one at load for the same reason — so registration is async.
 * A discovery failure registers the plain tool set only; registration
 * itself never throws into setup.
 */
export async function registerTools(
  toolDomain: ToolDomainLike,
  runtime: ToolRuntime,
  deps: { toolsEnabled: boolean; daemonTools?: boolean },
): Promise<string[]> {
  let boot: Discovery = { candidates: [], rung: 6, reason: "none", ambiguous: false };
  if (deps.daemonTools === undefined) {
    boot = await runtime.look().catch(() => boot);
  }
  const wantDaemon = deps.daemonTools ?? runtime.daemonToolsWanted(boot);
  const names: string[] = [];
  await toolDomain.transform((editor) => {
    editor.namespace({ name: NAMESPACE, description: NAMESPACE_DESCRIPTION });
    if (deps.toolsEnabled) {
      for (const t of runtime.tools()) {
        editor.add(t);
        names.push(`${NAMESPACE}_${t.name}`);
      }
    }
    if (wantDaemon) {
      for (const t of runtime.daemonTools()) {
        editor.add(t);
        names.push(`${NAMESPACE}_${t.name}`);
      }
    }
  });
  return names;
}
