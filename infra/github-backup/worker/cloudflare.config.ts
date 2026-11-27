import { bindings, defineConfig } from "cf/config";

export default defineConfig({
  worker: {
    name: "developing-today-backup",
    compatibilityDate: "2024-11-01",
    entrypoint: "src/index.js",
    env: {
      // Keep the existing namespace: this is the webhook queue drained by the laptop.
      GHB_EVENTS: bindings.kv({
        id: "46b1fe46e6434c5fa509556ed3a53ea2",
      }),
    },
  },
});
