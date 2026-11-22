import { bindings, defineConfig, exports } from "cf/config";
import * as entrypoint from "./src/index.ts" with { type: "cf-worker" };

// Deployment configuration for the existing Worker. Keep this aligned
// with wrangler.toml, which remains the local dev and Vitest config.
export default defineConfig({
  worker: {
    name: "atlas-search",
    compatibilityDate: "2025-09-15",
    entrypoint,
    workersDev: true,
    observability: { enabled: true },
    exports: {
      QueryCache: exports.durableObject({ storage: "sqlite" }),
    },
    env: {
      QUERY_CACHE: bindings.durableObject({ worker: "atlas-search", exportName: "QueryCache" }),
      SERVICE_BASE: bindings.text("https://atlas-search.security.cab"),
      GATE_TOKEN: bindings.secret(),
      SERVICE_TOKEN: bindings.secret(),
      ACCESS_CLIENT_ID: bindings.secret(),
      ACCESS_CLIENT_SECRET: bindings.secret(),
    },
  },
});
