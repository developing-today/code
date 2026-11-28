import { bindings, defineConfig, exports } from "cf/config";

export default defineConfig({
  worker: {
    name: "voice-line",
    compatibilityDate: "2025-09-15",
    entrypoint: "src/index.ts",
    workersDev: true,
    env: {
      TURSO_URL: bindings.text("https://voice-line-ledger-dezren39.aws-us-east-1.turso.io"),
      // The deployed commit the /version route names (#15793). This
      // config runs at build time in the release workflow, whose
      // upload path requires GITHUB_SHA (scripts/upload-version.mjs),
      // so each uploaded version names the commit it was built from;
      // a build without it answers "unknown", honestly.
      VERSION: bindings.text(process.env.GITHUB_SHA ?? "unknown"),
      TURSO_TOKEN: bindings.secret(),
      STATUS_INGEST_TOKEN: bindings.secret(),
      VOICE_SESSIONS: bindings.durableObject({
        worker: "voice-line",
        exportName: "VoiceSession",
      }),
      AI: bindings.ai({}),
    },
    exports: {
      VoiceSession: exports.durableObject({
        storage: "sqlite",
      }),
    },
  },
});
