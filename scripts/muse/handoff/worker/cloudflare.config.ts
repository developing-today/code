import { bindings, defineConfig, exports } from "cf/config";
import * as entrypoint from "./src/index.js" with { type: "cf-worker" };

// Deployment source for the cf Workers Versions flow. Keep this aligned with
// wrangler.jsonc, which remains the local-development/test configuration.
export default defineConfig({
  worker: {
    name: "muse-agent-handoff",
    compatibilityDate: "2026-11-21",
    entrypoint,
    workersDev: true,
    previewUrls: false,
    observability: { enabled: false },
    exports: {
      // cf expresses the existing SQLite class lifecycle here; the current
      // Wrangler config records its already-applied creation migration as v1.
      Handoff: exports.durableObject({ storage: "sqlite" }),
    },
    env: {
      HANDOFFS: bindings.durableObject({
        worker: "muse-agent-handoff",
        exportName: "Handoff",
      }),
      ADMIN_TOKEN: bindings.secret(),
      HANDOFF_ENCRYPTION_KEY: bindings.secret(),
    },
  },
});
