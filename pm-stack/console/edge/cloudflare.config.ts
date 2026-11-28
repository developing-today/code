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
      // CF-5 flip (owner ruling 2026-11-26 on #14866): the console ledgers
      // read from D1. Keep in sync with the vars in wrangler.jsonc.
      LEDGER_PAGES: bindings.text("on"),
      LEDGER_STORE: bindings.text("d1"),
      // The health check verifies the commit currently serving on workers.dev.
      VERSION: bindings.text(processEnv.GITHUB_SHA?.slice(0, 12) ?? "local"),
      // Service bindings, kept in sync with the services list in
      // wrangler.jsonc: the message bridge (#14373) and the
      // authenticated decision-events reader (CF-5a, #14967). A legacy
      // service environment would need hand resolution; neither uses one.
      BRIDGE: bindings.worker({ worker: "message-bridge" }),
      DECISION_EVENTS: bindings.worker({ worker: "decision-events" }),
      PM: bindings.durableObject({ worker: "pm-console-edge", exportName: "PmEdge" }),
      // The fleet-platform ledger's D1 home (#14968): keep in sync
      // with the d1_databases binding in wrangler.jsonc.
      LEDGER_DB: bindings.d1({ name: "fleet-platform", id: "57d15925-b0b8-4466-a92c-c200a6af4c45" }),
      ASSETS: bindings.assets(),
    },
  },
});
