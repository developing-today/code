import { cloudflareTest } from "@cloudflare/vitest-plugin";
import { defineConfig } from "vitest/config";

// The real-DO retention suite (test/worker.retention.test.ts) must run once
// per RETENTION_SECONDS value, because a Durable Object instance always sees
// the worker's configured vars — the value cannot be varied per test through
// SELF. Each project below boots the same worker with a different
// RETENTION_SECONDS; the test file itself stays value-agnostic (it reads the
// setting from the worker env and asserts a literal alarm constant).
const retentionSettings = ["0", "7d", "8640000000000", "3600"];

export default defineConfig({
  test: {
    projects: [
    {
      plugins: [
        cloudflareTest({
          wrangler: { configPath: "./wrangler.jsonc" },
        }),
      ],
      test: {
        include: ["test/worker.test.ts", "test/deploy-*.test.ts"],
        name: "default",
      },
    },
    ...retentionSettings.map((setting) => ({
      plugins: [
        cloudflareTest({
          wrangler: { configPath: "./wrangler.jsonc" },
          miniflare: {
            bindings: { RETENTION_SECONDS: setting },
          },
        }),
      ],
      test: {
        include: ["test/worker.retention.test.ts"],
        name: `retention-${setting}`,
      },
    })),
    ],
  },
});
