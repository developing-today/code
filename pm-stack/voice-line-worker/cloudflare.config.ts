import { bindings, defineConfig, exports } from "cf/config";

export default defineConfig({
	worker: {
		name: "voice-line",
		compatibilityDate: "2025-09-15",
		entrypoint: "src/index.ts",
		workersDev: true,
		env: {
			TURSO_URL: bindings.text("https://voice-line-ledger-dezren39.aws-us-east-1.turso.io"),
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
