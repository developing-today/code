import { defineWorkersConfig } from "@cloudflare/vitest-pool-workers/config";

export default defineWorkersConfig({
  test: {
    poolOptions: {
      workers: {
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
      },
    },
  },
});
