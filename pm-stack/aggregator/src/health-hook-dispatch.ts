export type HealthHookState = "down" | "up";

/** True once for each hook target/state transition, including after restart.
 *  With no recorded state (first run, or a state file from before hooks
 *  existed) an "up" is only a baseline: launching a hook then could record
 *  an outage from a probe the sentinel has not yet folded into "down", and
 *  no recovery hook would follow. The caller records the baseline. */
export function shouldDispatchHealthHook(
  dispatched: Record<string, HealthHookState>,
  target: string,
  state: HealthHookState,
): boolean {
  const previous = dispatched[target];
  if (previous === undefined) return state === "down";
  return previous !== state;
}

export function normalizeHookDispatchState(value: unknown): Record<string, HealthHookState> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const result: Record<string, HealthHookState> = {};
  for (const [target, state] of Object.entries(value)) {
    if (state === "down" || state === "up") result[target] = state;
  }
  return result;
}
