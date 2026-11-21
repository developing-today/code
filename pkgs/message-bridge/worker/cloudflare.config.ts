import { bindings, defineConfig, exports } from "cf/config";
import * as entrypoint from "./src/index.js" with { type: "cf-worker" };

// message-bridge Worker: deployment source of truth for the cf versions
// flow (deploy.mjs). Mirrors wrangler.jsonc, which only drives local
// `wrangler dev` now; keep the two in step. The Worker's secrets,
// ACCESS_TOKEN and CHATGPT_TOKEN, are never here: deploy.mjs uploads
// them with the version (full deploy) or carries the deployed ones
// forward (--code-only). PUBLIC_URL's host is a Workers custom domain
// that deploy.mjs attaches through the Workers domains API.
export default defineConfig({
  worker: {
    name: "message-bridge",
    compatibilityDate: "2026-10-25",
    entrypoint,
    exports: {
      MessageStore: exports.durableObject({ storage: "sqlite" }),
    },
    env: {
      MESSAGES: bindings.durableObject({ worker: "message-bridge", exportName: "MessageStore" }),
      PUBLIC_URL: bindings.text("https://bridge.security.cab"),
      WORKERS_DEV_URL: bindings.text("https://message-bridge.developing-today.workers.dev"),
      CF_VERSION_METADATA: bindings.versionMetadata(),
    },
  },
});
