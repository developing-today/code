import { bindings, defineConfig } from "cf/config";
import * as smoke from "./smoke.mjs" with { type: "cf-worker" };

export default defineConfig({
  worker: {
    name: "private-laptop-bridge-smoke",
    compatibilityDate: "2026-07-30",
    workersDev: false,
    previewUrls: false,
    domains: [],
    triggers: [],
    limits: { cpuMs: 10 },
    observability: { enabled: false, logs: { enabled: false }, traces: { enabled: false } },
    entrypoint: smoke,
    env: {
      ENABLED: bindings.text("false"),
      GATEWAY_HOST: bindings.text("private-bridge-smoke.invalid"),
      GATEWAY_ACCOUNT_ID: bindings.text(""),
      GATEWAY_POLICY_ID: bindings.text(""),
      START_MS: bindings.text(""),
      END_MS: bindings.text(""),
    },
  },
});
