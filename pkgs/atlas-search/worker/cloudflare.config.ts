import { bindings, defineConfig, exports } from "cf/config";
import * as entrypoint from "./src/index.ts" with { type: "cf-worker" };

// atlas-search Worker — deployment source of truth for the cf versions
// flow (see README.md). Mirrors wrangler.toml, which drives local dev
// and tests; keep the two in step. The Worker's secrets (GATE_TOKEN,
// SERVICE_TOKEN, ACCESS_CLIENT_ID, ACCESS_CLIENT_SECRET) are never
// here: fleet secrets hold them by name, and a deploy uploads them
// with the new version (cf workers versions create --secrets-file)
// so no version ever deploys without its gate and service tokens.
export default defineConfig({
  worker: {
    name: "atlas-search",
    compatibilityDate: "2025-09-15",
    entrypoint,
    exports: {
      QueryCache: exports.durableObject({ storage: "sqlite" }),
    },
    env: {
      QUERY_CACHE: bindings.durableObject({ worker: "atlas-search", exportName: "QueryCache" }),
      SERVICE_BASE: bindings.text("https://atlas-search.security.cab"),
      // D1 tenant hot cache (#9134 §5, #13100): mirrors the
      // wrangler.toml binding; placeholder id until the database
      // is created at deploy time. Flag default off.
      HOT_CACHE: bindings.d1({ name: "atlas-hot-cache", id: "00000000-0000-0000-0000-000000000001" }),
      HOT_CACHE_ENABLED: bindings.text("false"),
    },
  },
});
