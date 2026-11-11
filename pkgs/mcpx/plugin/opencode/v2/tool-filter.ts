/**
 * Per-session filtering of model-visible mcpx tools (#5122, item (1) of
 * #2841 — the mcp.transform deviation recorded on #5122).
 *
 * Item (1) as worded — register a filtered MCP server set per session via
 * `mcp.transform` — is not implementable on the pinned v2 surface: the
 * pinned editor (packages/plugin/src/promise/mcp.ts) is synchronous and
 * location-wide, with no session ID, and host reconciliation owns the
 * shared connections. The recorded equivalent is per-session filtering of
 * model-visible tool definitions through `session.hook("context")`:
 * the pinned SessionContext (packages/plugin/src/promise/session.ts)
 * carries `tools: Record<string, { description, input }>`, and core
 * (packages/core/src/session/model-request.ts) builds that record fresh
 * for every model request and drops any definition a hook removes, by
 * object identity. Deleting a key here therefore hides the tool from
 * that session's next model request only — it is NOT access control
 * (connections stay shared; gateway/shell calls can bypass it) and it
 * never mutates the location-wide MCP registry or any registered tool:
 * the record a hook sees is a per-request copy, rebuilt from the
 * registry on the next request.
 *
 * What counts as an mcpx tool: the model-facing names in the plugin's
 * own namespace — `<namespace>_<name>` with namespace `mcpx` (the
 * effectiveName rule in tools.ts), i.e. the `mcpx_` prefix. Only those
 * names can be hidden; a policy entry naming anything else is ignored,
 * so a malformed or hostile policy can never hide a host tool.
 *
 * Policy source: the plugin's per-session state in ctx.storage, key
 * `tool-filter:<sessionID>`, record `{ disabled: string[] }` — full
 * model-facing names (`mcpx_exec`). The production writer is the
 * plugin's user-facing `disabledTools` option (options.ts; env
 * MCPX_PLUGIN_DISABLED_TOOLS): index.ts records it as a session's
 * policy on the session's first sighting, unless the session already
 * has a policy of its own; `setDisabledTools` below is the write API
 * any other surface uses for a session-specific policy. The context hook is synchronous
 * and must never query the daemon (or storage) synchronously, so the
 * filter closes over an asynchronously refreshed snapshot, the
 * permission.ts pattern: setup refreshes a session's snapshot when the
 * session is first seen and whenever the hook fires, and `apply` uses
 * the latest snapshot. A policy recorded before a session's first turn
 * is therefore in force from that session's second turn at the latest;
 * a policy recorded mid-session takes effect on the turn after the
 * hook firing that refreshes it.
 */
import type { StorageLike } from "./remember.ts";

/** The plugin's tool namespace (tools.ts NAMESPACE), as a name prefix. */
export const MCPX_TOOL_PREFIX = "mcpx_";

const keyFor = (sessionID: string): string => `tool-filter:${sessionID}`;

/** The slice of the pinned SessionContext the filter uses. */
export type ToolFilterInput = {
  sessionID: string;
  tools?: Record<string, unknown>;
};

/**
 * Hide the disabled mcpx tools in one context-hook input, in place.
 * Returns the names removed. Only `mcpx_`-prefixed keys listed in
 * `disabled` are removed; every other key — and every surviving value,
 * by identity — is left exactly as the host built it.
 */
export function filterContextTools(input: ToolFilterInput, disabled: ReadonlySet<string>): string[] {
  if (!input.tools || disabled.size === 0) return [];
  const removed: string[] = [];
  for (const name of Object.keys(input.tools)) {
    if (!name.startsWith(MCPX_TOOL_PREFIX)) continue;
    if (!disabled.has(name)) continue;
    delete input.tools[name];
    removed.push(name);
  }
  return removed;
}

/**
 * Read a session's disabled-tools policy from storage. The record is
 * host-provided data, so its shape is not trusted (the remember.ts
 * discipline): anything but an object with a `disabled` array reads as
 * no policy, and non-string or non-mcpx entries are dropped, so a
 * malformed record can never hide a tool.
 */
export async function readDisabledTools(storage: StorageLike, sessionID: string): Promise<Set<string>> {
  return (await readDisabledToolsPolicy(storage, sessionID)) ?? new Set();
}

/**
 * Read a session's policy, distinguishing ABSENCE from an explicitly
 * empty policy (#5122 Codex re-review, PR #5170): `undefined` means
 * the session has no policy record of its own (nothing stored, or a
 * malformed record that does not carry a `disabled` array); any Set —
 * including the empty set from a stored `{ disabled: [] }` — is a
 * policy the session owns, and seeding defaults must never overwrite
 * it: `{ disabled: [] }` is how a session opts out of global
 * filtering. Entry filtering matches readDisabledTools.
 */
export async function readDisabledToolsPolicy(
  storage: StorageLike,
  sessionID: string,
): Promise<Set<string> | undefined> {
  const raw = await storage.get(keyFor(sessionID));
  if (raw === undefined || raw === null || typeof raw !== "object") return undefined;
  const list = (raw as { disabled?: unknown }).disabled;
  if (!Array.isArray(list)) return undefined;
  return new Set(list.filter((name): name is string => typeof name === "string" && name.startsWith(MCPX_TOOL_PREFIX)));
}

/** Record a session's disabled-tools policy (names are filtered on read). */
export async function setDisabledTools(
  storage: StorageLike,
  sessionID: string,
  disabled: Iterable<string>,
): Promise<void> {
  await storage.set(keyFor(sessionID), { disabled: [...disabled] });
}

export type ToolFilter = {
  /**
   * Refresh one session's snapshot from storage. Best-effort: a failed
   * read keeps the previous snapshot rather than un-hiding tools a
   * policy already hid.
   */
  refresh(sessionID: string): Promise<void>;
  /** Apply the session's latest snapshot to a context-hook input. */
  apply(input: ToolFilterInput): string[];
};

/**
 * The filter setup closes over: one snapshot per session, refreshed
 * asynchronously, applied synchronously by the context hook.
 */
export function createToolFilter(storage: StorageLike): ToolFilter {
  const snapshots = new Map<string, ReadonlySet<string>>();
  return {
    async refresh(sessionID) {
      const next = await readDisabledTools(storage, sessionID).catch(() => undefined);
      if (next !== undefined) snapshots.set(sessionID, next);
    },
    apply(input) {
      const disabled = snapshots.get(input.sessionID);
      if (!disabled) return [];
      return filterContextTools(input, disabled);
    },
  };
}
