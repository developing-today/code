import { bindings, defineConfig } from "cf/config";
import * as entrypoint from "./src/index.ts" with { type: "cf-worker" };

// desktop-commander Worker — deployment source of truth for the cf
// versions flow (see README.md). Mirrors wrangler.toml, kept in step
// as the human-readable config record. The Worker's secrets
// (ACCESS_CLIENT_ID, ACCESS_CLIENT_SECRET, MACHINE_BEARER) are never
// here: fleet secrets hold them by name, and a publish uploads them
// with the new version (cf workers versions create --secrets-file) so
// no version ever deploys without its credentials.
export default defineConfig({
  worker: {
    name: "desktop-commander",
    compatibilityDate: "2025-09-15",
    entrypoint,
    env: {
      BRIDGE_BASE: bindings.text("https://commander-link.security.cab"),
      ACCESS_TEAM_DOMAIN: bindings.text("snowy-sun-58a9.cloudflareaccess.com"),
      // Audience of the Access app scoped to /authorize.
      ACCESS_AUD: bindings.text("c6ddf0a5c0ba8742c0a3c74d12495bf5c4fe9bf6f01f604d62583cb2ed353879"),
      OAUTH_KV: bindings.kv({ id: "ea675d3c7d5b4f3b9f1cecae82e57d1b" }),
    },
  },
});
