import { bindings, defineConfig, exports } from "cf/config";
import { env as processEnv } from "node:process";
import * as entrypoint from "./src/worker.ts" with { type: "cf-worker" };

// Deploy-time config for the cf Workers Versions flow. wrangler.jsonc remains
// the local-dev source; keep its bindings and SQLite migration in sync here.
// The v1 new_sqlite_classes migration maps to the PmEdge SQLite export below.
// Deliberately no route is declared: expose.py assigns/removes it after health.
export default defineConfig({
  worker: {
    name: "pm-console-edge",
    compatibilityDate: "2026-10-01",
    compatibilityFlags: ["nodejs_compat"],
    entrypoint,
    workersDev: true,
    previewUrls: false,
    observability: { enabled: true },
    assets: {
      htmlHandling: "none",
      notFoundHandling: "none",
      runWorkerFirst: true,
    },
    exports: {
      PmEdge: exports.durableObject({ storage: "sqlite" }),
    },
    env: {
      PUBLIC_HOST: bindings.text("pm.security.cab"),
      // The health check verifies the commit currently serving on workers.dev.
      VERSION: bindings.text(processEnv.GITHUB_SHA?.slice(0, 12) ?? "local"),
      PM: bindings.durableObject({ worker: "pm-console-edge", exportName: "PmEdge" }),
      // The fleet-platform ledger's D1 home (#14968): keep in sync
      // with the d1_databases binding in wrangler.jsonc.
      LEDGER_DB: bindings.d1({ name: "fleet-platform", id: "57d15925-b0b8-4466-a92c-c200a6af4c45" }),
      ASSETS: bindings.assets(),
    },
  },
});
