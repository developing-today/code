import { cloudflareTest } from "@cloudflare/vitest-pool-workers";
import { defineConfig } from "vitest/config";

// Vitest 4 / @cloudflare/vitest-pool-workers 0.23: the pool is registered
// as a Vite plugin; the old defineWorkersConfig + test.poolOptions.workers
// shape (and the /config export it came from) no longer exists. Mirrors the
// migration already landed for pkgs/github-ci/worker.
export default defineConfig({
  plugins: [
    cloudflareTest({
      wrangler: { configPath: "./wrangler.toml" },
      miniflare: {
        bindings: {
GATE_TOKEN: "test-gate",
            SERVICE_BASE: "https://service.test",
            SERVICE_TOKEN: "test-service-token",
            ACCESS_CLIENT_ID: "test-access-id",
            ACCESS_CLIENT_SECRET: "test-access-secret",
        },
      },
    }),
  ],
});
