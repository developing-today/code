import { bindings, defineConfig, exports } from "cf/config";
import * as entrypoint from "./src/index.js" with { type: "cf-worker" };

const workerName = process.env.MUSE_HANDOFF_WORKER_NAME ?? "muse-agent-handoff";

// Deployment source for the cf Workers Versions flow. Keep this aligned with
// wrangler.jsonc, which remains the local-development/test configuration.
export default defineConfig({
  worker: {
    name: workerName,
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
        worker: workerName,
        exportName: "Handoff",
      }),
      // The deployed commit the /version route names (#15793). This
      // config runs at build time: both jobs of the bespoke release
      // workflow (muse-handoff-upload.yml) build from the checked-out
      // commit, whose SHA Actions provides as GITHUB_SHA, so each
      // created version names the commit it was built from; a build
      // without it answers "unknown", honestly.
      VERSION: bindings.text(process.env.GITHUB_SHA ?? "unknown"),
      ADMIN_TOKEN: bindings.secret(),
      HANDOFF_ENCRYPTION_KEY: bindings.secret(),
    },
  },
});
