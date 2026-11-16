import { cloudflareTest } from "@cloudflare/vitest-pool-workers";
import { defineConfig } from "vitest/config";

// Vitest 4 / @cloudflare/vitest-pool-workers 0.23: the pool is registered
// as a Vite plugin; the old defineWorkersConfig + test.poolOptions.workers
// shape (and the /config export it came from) no longer exists.
export default defineConfig({
  plugins: [
    cloudflareTest({
      wrangler: { configPath: "./wrangler.toml" },
      miniflare: {
        bindings: {
          GITHUB_WEBHOOK_SECRET: "test-webhook-secret",
          TURSO_PLATFORM_TOKEN: "test-platform-token",
          TURSO_QUEUE_TOKEN: "test-queue-token",
          EVENTS_BROKER_URL: "",
        },
      },
    }),
  ],
});
