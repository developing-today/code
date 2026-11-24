import { bindings, defineConfig, exports } from "cf/config";
import * as entrypoint from "./dist/worker.js" with { type: "cf-worker" };

// Deployment configuration used by the repository-pinned cf CLI. Keep it
// aligned with wrangler.toml (local dev and the existing Worker settings);
// test/config-parity.test.mjs pins the shared binding and migration fields.
export default defineConfig({
  worker: {
    name: "cold-store-rehydrator",
    compatibilityDate: "2026-10-25",
    workersDev: true,
    entrypoint,
    exports: {
      // Existing v1 migration: retain the provisioned SQLite namespace.
      BundleFlight: exports.durableObject({ storage: "sqlite" }),
    },
    env: {
      MANIFEST: bindings.d1({
        id: "15d5f460-835c-4f24-b229-222908a5b30c",
        name: "cold-store-manifest",
      }),
      BUNDLE_FLIGHT: bindings.durableObject({
        worker: "cold-store-rehydrator",
        exportName: "BundleFlight",
      }),
      // The CLI carries forward the deployed secret values into versions.
      // Names are declared here; no secret values are committed or read by CI.
      REHYDRATOR_API_KEY: bindings.secret(),
      GOOGLE_CLIENT_ID: bindings.secret(),
      GOOGLE_CLIENT_SECRET: bindings.secret(),
      GOOGLE_REFRESH_TOKEN: bindings.secret(),
    },
  },
});
