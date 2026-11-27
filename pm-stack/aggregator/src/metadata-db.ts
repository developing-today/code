import { join } from "node:path";
import { requireHome } from "./home.ts";

/** Resolve the same SQLite path as Fleet's SqliteMetadataStore::default_path. */
export function fleetMetadataDbPath(env: Record<string, string | undefined> = process.env): string {
  const override = env.FLEET_METADATA_DB;
  if (override !== undefined && override.trim() !== "") return override;

  const xdgStateHome = env.XDG_STATE_HOME;
  if (xdgStateHome !== undefined && xdgStateHome.trim() !== "" && xdgStateHome.startsWith("/")) {
    return join(xdgStateHome, "fleet", "metadata.db");
  }

  const home = requireHome("FLEET_METADATA_DB", env);
  if (!home.startsWith("/")) throw new Error("HOME must be absolute to match Fleet metadata database resolution");
  return join(home, ".local", "state", "fleet", "metadata.db");
}
