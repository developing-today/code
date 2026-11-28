import { bindings, defineConfig } from "cf/config";
import * as entrypoint from "./src/index.ts" with { type: "cf-worker" };

// mcpx Worker — deployment source of truth for the cf
// versions flow (see README.md). Mirrors wrangler.toml, kept in step
// as the human-readable config record. The Worker's secrets
// (ACCESS_CLIENT_ID, ACCESS_CLIENT_SECRET, MACHINE_BEARER) are never
// here: fleet secrets hold them by name, and a publish uploads them
// with the new version (cf workers versions create --secrets-file) so
// no version ever deploys without its credentials.
export default defineConfig({
  worker: {
    name: "mcpx",
    compatibilityDate: "2025-09-15",
    entrypoint,
    env: {
      BRIDGE_BASE: bindings.text("https://mcpx-link.security.cab"),
      ACCESS_TEAM_DOMAIN: bindings.text("snowy-sun-58a9.cloudflareaccess.com"),
      // Audience of the Access app scoped to /authorize.
      ACCESS_AUD: bindings.text("b34b9764fb317774bf5c0ff0ac9cbd8a2ab80ed346d06f08d74967011bac640e"),
      OAUTH_KV: bindings.kv({ id: "11eb93dc9fa24087ae57b12483dc6172" }),
    },
  },
});
