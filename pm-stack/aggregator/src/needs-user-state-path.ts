import { join } from "node:path";

/** Resolve the question store using the currently deployed legacy name. */
export function needsUserStatePath(
  env: Record<string, string | undefined>,
  stateDirectory: () => string,
): string {
  const legacyOverride = env.PM_AGG_NEEDS_DREW_FILE?.trim();
  return legacyOverride || join(stateDirectory(), "needs-drew.json");
}
