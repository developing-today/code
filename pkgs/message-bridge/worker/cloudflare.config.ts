import { bindings, defineConfig, exports, triggers } from "cf/config";
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
    // Bridge state backup (#14951): one scheduled export a day of the
    // whole MessageStore into the BRIDGE_BACKUP bucket (backup.js).
    triggers: [triggers.scheduled({ schedule: "17 8 * * *" })],
    exports: {
      MessageStore: exports.durableObject({ storage: "sqlite" }),
    },
    env: {
      MESSAGES: bindings.durableObject({ worker: "message-bridge", exportName: "MessageStore" }),
      BRIDGE_BACKUP: bindings.r2({ name: "message-bridge-backup" }),
      PUBLIC_URL: bindings.text("https://bridge.security.cab"),
      WORKERS_DEV_URL: bindings.text("https://message-bridge.developing-today.workers.dev"),
      CONSOLE_URL: bindings.text("https://pm.security.cab/console/#more/messages"),
      // The /version stamp (#15793): the commit being deployed, read
      // at build time; the Actions deploy sets GITHUB_SHA (deploy.mjs
      // derives its version tag from the same value).
      VERSION: bindings.text(process.env.GITHUB_SHA ?? "unknown"),
      CF_VERSION_METADATA: bindings.versionMetadata(),
    },
  },
});
