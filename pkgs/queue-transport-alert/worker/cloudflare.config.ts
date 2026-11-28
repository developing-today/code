import { bindings, defineConfig, exports, triggers } from "cf/config";
import * as entrypoint from "./src/index.js" with { type: "cf-worker" };

// queue-transport-alert Worker (code#6369): the deploy source of truth
// for the cf versions flow (deploy.mjs, `nix run
// .#queue-transport-alert-deploy`). wrangler.jsonc mirrors it for local
// `wrangler dev`; test/config.test.mjs keeps the two in step.
export default defineConfig({
  worker: {
    name: "queue-transport-alert",
    compatibilityDate: "2026-10-25",
    entrypoint,
    observability: { enabled: true },
    triggers: [triggers.scheduled({ schedule: "*/10 * * * *" })],
    exports: {
      // Created by the wrangler `new_sqlite_classes` migration (tag v1);
      // declared here so the namespace and its state carry over.
      TransportState: exports.durableObject({ storage: "sqlite" }),
    },
    env: {
      TRANSPORT_STATE: bindings.durableObject({ worker: "queue-transport-alert", exportName: "TransportState" }),
      BROKER_URL: bindings.text("https://queue-broker.security.cab"),
      REPO: bindings.text("developing-today/code"),
      THRESHOLD: bindings.text("2"),
      PROBE_TIMEOUT_MS: bindings.text("10000"),
      QUEUE: bindings.text("repo-health-code-pilot"),
      // Drew's Telegram user id (DM with @shed_openclaw_bot); also in home/openclaw.
      TELEGRAM_CHAT_ID: bindings.text("8290042325"),
      PUBLIC_URL: bindings.text("https://queue-transport-alert.developing-today.workers.dev"),
      // "true" files/comments on incidents, pushes and enqueues; anything else only logs.
      APPLY: bindings.text("true"),
      // Uploaded with each version by deploy.mjs (--secrets-file from
      // stdin); a secret missing from that upload inherits the previous
      // version's value.
      ACCESS_CLIENT_ID: bindings.secret(),
      ACCESS_CLIENT_SECRET: bindings.secret(),
      GITHUB_TOKEN: bindings.secret(),
      TELEGRAM_BOT_TOKEN: bindings.secret(),
      BROKER_AGENT_TOKEN: bindings.secret(),
    },
  },
});
