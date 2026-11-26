import { bindings, defineConfig, exports as cfExports } from "cf/config";
import { readFileSync } from "node:fs";

import * as entrypoint from "./src/index.js" with { type: "cf-worker" };

const config = JSON.parse(readFileSync(new URL("./config.json", import.meta.url), "utf8"));
const mailConfig = {
  defaultTransport: config.defaultTransport,
  transports: config.transports,
  limits: config.limits,
  templates: config.templates,
};

export default defineConfig({
  worker: {
    name: config.worker,
    compatibilityDate: config.compatibilityDate,
    entrypoint,
    exports: {
      Limiter: cfExports.durableObject({ state: "created", storage: "sqlite" }),
    },
    env: {
      MAIL_CONFIG: bindings.text(JSON.stringify(mailConfig)),
      EMAIL: bindings.sendEmail({ allowedSenderAddresses: config.senders }),
      LIMITER: bindings.durableObject({ worker: config.worker, exportName: "Limiter" }),
      CALLERS: bindings.secret(),
    },
  },
});
