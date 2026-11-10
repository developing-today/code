/**
 * The daemon picker for the v2 tree (#1858 SET-REMAINDER item (3),
 * implemented under #2841).
 *
 * Item (3) asked for a TUI `select()` daemon picker: a promise-based
 * picker where the pinned v2 surface supports it, the choice persisted
 * via ctx.storage (remember.ts). The pinned surface does NOT support
 * it — verified 2026-11-08 against the pinned opencode fork
 * (~/git/opencode at ededbacbfd, @opencode/plugin version 2.0.30):
 *
 *  - packages/plugin/src/promise/plugin.ts — the setup `Context` has
 *    a fixed domain list (agent, aisdk, command, credential, event,
 *    integration, mcp, messaging, model, generate, permission, plugin,
 *    provider, reference, rpc, session, shell, skill, storage, tool,
 *    vcs, websearch, worktree). There is no ui/tui/question domain,
 *    and no select/dialog/prompt method on any domain a server plugin
 *    receives. (The `question` tool is a built-in model tool with
 *    `codemode: false`; it is not a plugin surface.)
 *  - The v1 picker (mcpx-tui.tsx) works only because v1 has a separate
 *    TUI plugin realm (`@opencode-ai/plugin/tui`, DialogSelect). The
 *    v2 module contract ({ id, setup } / { id, effect }) has no TUI
 *    counterpart in the pin.
 *
 * So this module lands the closest supported equivalent, and the
 * deviation is recorded on #2841 and #1858:
 *
 *  - The picker is a tool, `mcpx_daemon_pick`. Called with no index it
 *    renders the picker menu — the ranked candidates, the in-use one
 *    marked, each with the config path that actually distinguishes
 *    daemons (the same options v1's DialogSelect showed). Called with
 *    an index it makes the choice.
 *  - The choice goes through the one interactive channel v2 server
 *    plugins do have: the permission domain. The selection asserts
 *    `mcpx_daemon_select` (permission.ts) with the same payload
 *    daemon_select sends, so the user confirms the exact daemon
 *    before anything is applied — the promise a TUI select's onSelect
 *    would have resolved through.
 *  - Cancellation is a picker outcome, not an error: a rejected assert
 *    (the user said no, or a deny rule fired) applies nothing and the
 *    tool reports the choice was not changed. Invalid indexes and
 *    picker errors likewise never touch the current selection. Only a
 *    confirmed selection reaches applyChoice, which persists through
 *    ctx.storage via remember.ts (applyChoice in tools.ts).
 *
 * The core here is host-free: candidates in, an injected assert and
 * apply in, a discriminated outcome out — so the stub-host tests are
 * the contract, exactly as with the rest of the v2 tree.
 */
import { ago, label, targetKey, type Candidate, type Target } from "../../core/daemon.ts";
import { DAEMON_SELECT_ACTION, type PermissionAssertInput } from "./permission.ts";

export type PickerScope = "session" | "until-gone" | "indefinite" | "permanent";

/** One row of the picker menu — v1's DialogSelect option, as data. */
export type PickerOption = {
  index: number;
  title: string;
  /** The config path first: it is what actually distinguishes two daemons. */
  description: string;
  /** targetKey of the candidate — the value a selection names. */
  value: string;
  /** The daemon this session is currently using (DialogSelect's `current`). */
  current: boolean;
  candidate: Candidate;
};

/** The detail suffix v1's picker showed beside the config path. */
const detail = (c: Candidate): string => {
  const bits: string[] = [];
  if (c.startedAt) bits.push(`started ${ago(c.startedAt)}`);
  if (c.servers !== undefined) bits.push(`${c.servers} servers`);
  if (c.sessions) bits.push(`${c.sessions} sessions`);
  if (c.version) bits.push(c.version);
  return bits.join(" · ");
};

/**
 * Build the picker options from an already-ranked candidate list
 * (rank() in daemon.ts, the same order daemon_status prints and
 * daemon_select indexes). `current` marks the daemon in use.
 */
export function buildPickerOptions(candidates: Candidate[], current?: Target): PickerOption[] {
  return candidates.map((candidate, index) => {
    const extra = detail(candidate);
    return {
      index,
      title: label(candidate),
      description: `${candidate.configPath ?? targetKey(candidate.target)}${extra ? "  —  " + extra : ""}`,
      value: targetKey(candidate.target),
      current: current !== undefined && targetKey(current) === targetKey(candidate.target),
      candidate,
    };
  });
}

/**
 * Render the picker menu as text — what the user sees in place of the
 * TUI dialog the pin cannot offer. The footer names the one supported
 * way to choose, so the menu is never a dead end.
 */
export function renderPickerMenu(candidates: Candidate[], current?: Target): string {
  const options = buildPickerOptions(candidates, current);
  if (options.length === 0) {
    return "No mcpx daemon is running. Start one with `mcpx daemon`, or set MCPX_DAEMON_ENDPOINT.";
  }
  const head =
    `mcpx daemon picker — ${options.length} daemon${options.length === 1 ? "" : "s"} running, best match first. ` +
    `* marks the one in use.\n\n`;
  const rows = options
    .map((o) => `${o.current ? "*" : " "} [${o.index}] ${o.title}\n      ${o.description}`)
    .join("\n\n");
  const foot =
    `\n\nCall mcpx_daemon_pick with candidate=<index> to choose; ` +
    `the user confirms before anything changes, and cancelling changes nothing.`;
  return head + rows + foot;
}

export type PickerOutcome =
  | { applied: true; candidate: Candidate; note: string; text: string }
  | {
      applied: false;
      reason: "no_candidates" | "invalid_index" | "no_permission" | "cancelled";
      text: string;
    };

export type PickerSelectionDeps = {
  sessionID: string;
  /** The ranked candidates, as buildPickerOptions received them. */
  candidates: Candidate[];
  current?: Target;
  index: number;
  scope: PickerScope;
  stopOthers?: boolean;
  /**
   * The permission assert (permission.ts via tools.ts). Absent means
   * this host offers no permission domain: the picker refuses rather
   * than applying a choice unconfirmed — daemon_select's rule.
   */
  assertPermission?: (input: PermissionAssertInput) => Promise<void>;
  /** Apply a confirmed choice (tools.ts applyChoice: storage-backed). */
  applyChoice: (target: Target, scope: PickerScope) => Promise<string>;
};

/**
 * Make one picker selection. The invariant the tests hold: applyChoice
 * runs only after a successful assert, with exactly the picked target
 * — every other path (no candidates, bad index, no permission domain,
 * a rejected assert) returns applied:false and mutates nothing.
 */
export async function runPickerSelection(deps: PickerSelectionDeps): Promise<PickerOutcome> {
  const { candidates, index, scope } = deps;
  if (candidates.length === 0) {
    return { applied: false, reason: "no_candidates", text: "No daemon to pick. Run mcpx_daemon_status first." };
  }
  const pick = candidates[index];
  if (!pick) {
    return {
      applied: false,
      reason: "invalid_index",
      text: `No candidate [${index}]. The picker lists 0..${candidates.length - 1}.`,
    };
  }
  if (!deps.assertPermission) {
    return {
      applied: false,
      reason: "no_permission",
      text:
        "This host offers no permission prompt, so the choice was not applied. " +
        "Start the session where mcpx_daemon_pick can ask, or set the daemon " +
        "endpoint in the mcpx config directly.",
    };
  }
  try {
    await deps.assertPermission({
      sessionID: deps.sessionID,
      action: DAEMON_SELECT_ACTION,
      resources: [targetKey(pick.target)],
      save: [targetKey(pick.target)],
      metadata: {
        daemon: label(pick),
        socket: targetKey(pick.target),
        config: pick.configPath,
        started: pick.startedAt,
        version: pick.version,
        servers: pick.servers,
        sessions: pick.sessions,
        remember: scope,
        stopOthers: deps.stopOthers === true,
        others: candidates.filter((c) => c !== pick).map((c) => targetKey(c.target)),
        picker: true,
      },
    });
  } catch (err) {
    // The user cancelled the picker (or a deny rule fired): a normal
    // picker outcome. Nothing was applied; say so plainly instead of
    // failing the tool call the way a raw rejection would read.
    return {
      applied: false,
      reason: "cancelled",
      text:
        `Picker cancelled (${err instanceof Error ? err.message : String(err)}). ` +
        `The daemon choice was not changed${deps.current ? ` — still using ${targetKey(deps.current)}` : ""}.`,
    };
  }
  const note = await deps.applyChoice(pick.target, scope);
  return {
    applied: true,
    candidate: pick,
    note,
    text: `Now using ${label(pick)} (${targetKey(pick.target)}). ${note}`,
  };
}
