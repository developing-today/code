import { join } from "node:path";
import { requireHome } from "./home.ts";

/** Resolve the PM metadata database path. */
export function fleetMetadataDbPath(env: Record<string, string | undefined> = process.env): string {
  return env.PM_METADATA_DB_PATH ?? join(requireHome("PM_METADATA_DB_PATH", env), ".local/state/pm-aggregator/pm-metadata.sqlite");
}
