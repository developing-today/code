import { bindings, defineConfig, triggers } from "cf/config";

// The Wrangler config remains authoritative for deployment migrations. The
// Build Output config below mirrors its non-migration Worker settings; `cf
// workers versions create` refuses to apply a pending migration.
export default defineConfig({
  worker: {
    name: "lab-ops-hub",
    compatibilityDate: "2025-09-01",
    compatibilityFlags: ["nodejs_compat"],
    entrypoint: "src/index.ts",
    observability: { enabled: true },
    assets: { runWorkerFirst: ["/api/*"] },
    triggers: [
      triggers.scheduled({ schedule: "23 4 * * *" }),
      triggers.queue({ maxBatchSize: 10, maxRetries: 3, name: "labops-jobs" }),
    ],
    env: {
      DB: bindings.d1({ name: "labops-db", id: "3115e67f-150e-4ee8-8b11-216127901ae8" }),
      FILES: bindings.r2({ name: "labops-files" }),
      CACHE: bindings.kv({ id: "ac69d54d9e3f455d99a940b3fced4b76" }),
      JOBS: bindings.queue({ name: "labops-jobs" }),
      LIVE: bindings.durableObject({ worker: "lab-ops-hub", exportName: "LiveRoom" }),
      CRAWLER: bindings.durableObject({ worker: "lab-ops-hub", exportName: "CrawlerContainerV2" }),
      AI: bindings.ai({}),
      ASSETS: bindings.assets(),
    },
    unsafe: {
      metadata: {
        // `cf migrate` does not migrate Containers. Keep this manual mapping
        // aligned with wrangler.jsonc and container-image.nix's immutable v5.
        containers: [
          {
            class_name: "CrawlerContainerV2",
            image: "registry.cloudflare.com/234b866b99ce2b142d7963d8d139cbfa/lab-ops-hub-crawlercontainer:v5",
            max_instances: 2,
            instance_type: "lite",
          },
        ],
      },
    },
  },
});
