import { bindings, defineConfig, exports } from "cf/config";
import * as entrypoint from "./src/index.js" with { type: "cf-worker" };

// message-bridge Worker: deployment source of truth for the cf versions
// flow (deploy.mjs). Mirrors wrangler.jsonc, which only drives local
// `wrangler dev` now; keep the two in step. The Worker's one secret,
// ACCESS_TOKEN, is never here: deploy.mjs uploads it with the version
// (full deploy) or carries the deployed one forward (--code-only).
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
      PUBLIC_URL: bindings.text("https://message-bridge.developing-today.workers.dev"),
      CF_VERSION_METADATA: bindings.versionMetadata(),
    },
  },
});
