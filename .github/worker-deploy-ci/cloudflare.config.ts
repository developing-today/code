import { bindings, defineConfig } from "cf/config";
import * as entrypoint from "../../pkgs/github-ci/worker/src/index.ts" with { type: "cf-worker" };

export default defineConfig({
  worker: {
    name: "github-ci",
    compatibilityDate: "2025-09-15",
    workersDev: true,
    entrypoint,
    env: {
      TURSO_PLATFORM_URL: bindings.text("https://fleet-platform-dezren39.aws-us-east-1.turso.io"),
      TURSO_QUEUE_URL: bindings.text("https://fleet-queue-dezren39.aws-us-east-1.turso.io"),
      EVENTS_BROKER_URL: bindings.text("https://queue-broker.security.cab"),
      GITHUB_WEBHOOK_SECRET: bindings.secret(),
      TURSO_PLATFORM_TOKEN: bindings.secret(),
      TURSO_QUEUE_TOKEN: bindings.secret(),
      EVENTS_BROKER_TOKEN: bindings.secret(),
      EVENTS_BROKER_ACCESS_CLIENT_ID: bindings.secret(),
      EVENTS_BROKER_ACCESS_CLIENT_SECRET: bindings.secret(),
    },
  },
});
