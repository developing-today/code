import { bindings, defineConfig, exports } from "cf/config";
import * as entrypoint from "./src/index.ts" with { type: "cf-worker" };

// pkarr Worker — deployment source of truth for the cf versions flow
// (see README.md). Mirrors wrangler.jsonc, which drives local dev and
// tests; keep the two in step.
export default defineConfig({
  worker: {
    name: "pkarr",
    compatibilityDate: "2026-10-01",
    entrypoint,
    exports: {
      PkarrStore: exports.durableObject({ storage: "sqlite" }),
    },
    env: {
      PKARR_STORE: bindings.durableObject({ worker: "pkarr", exportName: "PkarrStore" }),
      PUT_LIMITER: bindings.rateLimit({ namespace: "7101", simple: { limit: 60, period: 60 } }),
      GET_LIMITER: bindings.rateLimit({ namespace: "7102", simple: { limit: 300, period: 60 } }),
      DOH_LIMITER: bindings.rateLimit({ namespace: "7103", simple: { limit: 300, period: 60 } }),
      ORIGIN: bindings.text("pkarr.developing-today.workers.dev"),
      RETENTION_SECONDS: bindings.text("604800"),
      DNS_TTL: bindings.text("30"),
    },
  },
});
