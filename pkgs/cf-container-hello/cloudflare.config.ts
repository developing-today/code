// CI-only cf Build Output source. The operator path remains wrangler.jsonc and
// `npm run deploy`; this native config exists only for credential-free parity
// checks before a Versions staging dry-run.
import { bindings, defineConfig, defineContainer, exports } from "cf/config";

const hello = defineContainer({
  name: "cf-container-hello-hello",
  schedulingPolicy: "durable-object",
  images: {
    v1: { reference: "registry.cloudflare.com/234b866b99ce2b142d7963d8d139cbfa/hello-world:v1" },
  },
});
const crawler = defineContainer({
  name: "cf-container-hello-crawler-probe",
  schedulingPolicy: "durable-object",
  images: {
    v5: { reference: "registry.cloudflare.com/234b866b99ce2b142d7963d8d139cbfa/lab-ops-hub-crawlercontainer:v5" },
  },
});
const lite = defineContainer({
  name: "cf-container-hello-lite",
  schedulingPolicy: "durable-object",
  images: {
    v1: { reference: "registry.cloudflare.com/234b866b99ce2b142d7963d8d139cbfa/hello-world:v1" },
  },
});

export default defineConfig({
  worker: {
    name: "cf-container-hello",
    compatibilityDate: "2026-10-01",
    entrypoint: "src/index.ts",
    observability: { enabled: true },
    env: {
      HELLO_CONTAINER: bindings.durableObject({ worker: "cf-container-hello", exportName: "HelloContainer" }),
      CRAWLER_PROBE: bindings.durableObject({ worker: "cf-container-hello", exportName: "CrawlerProbe" }),
      HELLO_LITE: bindings.durableObject({ worker: "cf-container-hello", exportName: "HelloLite" }),
    },
    exports: {
      HelloContainer: exports.durableObject({ storage: "sqlite", container: hello }),
      CrawlerProbe: exports.durableObject({ storage: "sqlite", container: crawler }),
      HelloLite: exports.durableObject({ storage: "sqlite", container: lite }),
    },
    // Preserve the two migration groups from the operator Wrangler config in
    // effective Build Output metadata for the parity gate.
    unsafe: {
      metadata: {
        containers: [
          {
            class_name: "HelloContainer",
            image: "registry.cloudflare.com/234b866b99ce2b142d7963d8d139cbfa/hello-world:v1",
            max_instances: 2,
          },
          {
            class_name: "CrawlerProbe",
            image: "registry.cloudflare.com/234b866b99ce2b142d7963d8d139cbfa/lab-ops-hub-crawlercontainer:v5",
            max_instances: 1,
          },
          {
            class_name: "HelloLite",
            image: "registry.cloudflare.com/234b866b99ce2b142d7963d8d139cbfa/hello-world:v1",
            instance_type: "lite",
            max_instances: 1,
          },
        ],
        migrations: [
          { new_sqlite_classes: ["HelloContainer"], tag: "v1" },
          { new_sqlite_classes: ["CrawlerProbe", "HelloLite"], tag: "v2" },
        ],
      },
    },
  },
  containers: [hello, crawler, lite],
});
