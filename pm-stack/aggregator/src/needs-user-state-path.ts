import { existsSync } from "node:fs";
import { join } from "node:path";

/** Resolve the question store using the currently deployed legacy name. */
export function needsUserStatePath(
  env: Record<string, string | undefined>,
  stateDirectory: () => string,
  exists: (path: string) => boolean = existsSync,
): string {
  const needsUserOverride = env.PM_AGG_NEEDS_USER_FILE?.trim();
  if (needsUserOverride) return needsUserOverride;
  const legacyOverride = env.PM_AGG_NEEDS_DREW_FILE?.trim();
  if (legacyOverride) return legacyOverride;

  const directory = stateDirectory();
  const needsUserPath = join(directory, "needs-user.json");
  const legacyPath = join(directory, "needs-drew.json");
  if (exists(needsUserPath)) return needsUserPath;
  if (exists(legacyPath)) return legacyPath;
  return needsUserPath;
}
