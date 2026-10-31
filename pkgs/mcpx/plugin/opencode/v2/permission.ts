/**
 * The v2 permission gate for mcpx's own actions (#1858 SET-REMAINDER
 * item (2), implemented under #2841).
 *
 * v1's daemon_select confirmed through ctx.ask — the only interactive
 * channel a v1 server plugin had. The v2 replacement is the permission
 * domain (verified against the pinned source,
 * packages/plugin/src/promise/permission.ts):
 *
 *  - `permission.assert(input)` evaluates the session's rules for an
 *    action and, when approval is required, prompts the user and waits.
 *    It resolves when allowed; a deny rule or a rejection rejects.
 *  - `permission.hook("evaluate", fn)` receives every evaluation with
 *    the rule-computed effect already set, and may override it — the
 *    host's own config policy plugin uses it exactly this way
 *    (packages/core/src/config/plugin/policy.ts).
 *
 * mcpx's policy is narrow: its one user-confirmation action,
 * `mcpx_daemon_select`, always evaluates to "ask", so a blanket allow
 * rule can never make daemon_select apply a choice silently — the
 * failure mode the v2 core shipped with. A deny the host computed is
 * left alone (the core short-circuits denies before hooks run, but the
 * handler does not rely on that), and every other action is untouched.
 * The assertion itself carries `save: [target]` as v1's ask carried
 * `always: [target]`, so the host can record the user's answer.
 */

/** The permission action daemon_select asserts, as v1 named it. */
export const DAEMON_SELECT_ACTION = "mcpx_daemon_select";

/** The slice of the pinned PermissionEvaluation this gate uses. */
export type PermissionEvaluationLike = {
  sessionID: string;
  action: string;
  resources: ReadonlyArray<string>;
  effect: "allow" | "deny" | "ask";
  message?: string;
};

/** The assert payload, in the pinned AssertInput shape. */
export type PermissionAssertInput = {
  sessionID: string;
  action: string;
  resources: string[];
  save?: string[];
  metadata?: Record<string, unknown>;
};

/** The slice of the v2 permission domain this plugin uses. */
export type PermissionDomainLike = {
  hook(
    name: "evaluate",
    fn: (input: PermissionEvaluationLike) => void,
  ): Promise<unknown>;
  assert(input: PermissionAssertInput): Promise<void>;
};

/**
 * The evaluate hook: mcpx's own confirmation action always asks.
 * Anything else — other actions, a computed deny — passes through
 * unchanged.
 */
export function evaluateMcpxPermission(event: PermissionEvaluationLike): void {
  if (event.action !== DAEMON_SELECT_ACTION) return;
  if (event.effect === "deny") return;
  event.effect = "ask";
}
