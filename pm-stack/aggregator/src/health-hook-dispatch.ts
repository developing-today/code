export type HealthHookState = "down" | "up";

/** True once for each hook target/state transition, including after restart. */
export function shouldDispatchHealthHook(
  dispatched: Record<string, HealthHookState>,
  target: string,
  state: HealthHookState,
): boolean {
  return dispatched[target] !== state;
}

export function normalizeHookDispatchState(value: unknown): Record<string, HealthHookState> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const result: Record<string, HealthHookState> = {};
  for (const [target, state] of Object.entries(value)) {
    if (state === "down" || state === "up") result[target] = state;
  }
  return result;
}
