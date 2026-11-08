import { defineWorkersConfig } from "@cloudflare/vitest-pool-workers/config";

export default defineWorkersConfig({
  test: {
    poolOptions: {
      workers: {
        wrangler: { configPath: "./wrangler.toml" },
        miniflare: {
          bindings: {
            GITHUB_WEBHOOK_SECRET: "test-webhook-secret",
            TURSO_PLATFORM_TOKEN: "test-platform-token",
            TURSO_QUEUE_TOKEN: "test-queue-token",
            EVENTS_BROKER_URL: "",
          },
        },
      },
    },
  },
});
