import { bindings, defineConfig } from "cf/config";
import * as entrypoint from "./src/index.ts" with { type: "cf-worker" };

// desktop-commander Worker — deployment source of truth for the cf
// versions flow (see README.md). Mirrors wrangler.toml, kept in step
// as the human-readable config record. The Worker's secrets
// (ACCESS_CLIENT_ID, ACCESS_CLIENT_SECRET) are never here: fleet
// secrets hold them by name, and a publish uploads them with the new
// version (cf workers versions create --secrets-file) so no version
// ever deploys without its bridge credentials.
export default defineConfig({
  worker: {
    name: "desktop-commander",
    compatibilityDate: "2025-09-15",
    entrypoint,
    env: {
      BRIDGE_BASE: bindings.text("https://commander-link.security.cab"),
    },
  },
});
