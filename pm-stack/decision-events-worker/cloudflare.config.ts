import { bindings, defineConfig, exports } from "cf/config";

export default defineConfig({
	worker: {
		name: "decision-events",
		compatibilityDate: "2025-09-15",
		entrypoint: "src/index.ts",
		workersDev: true,
		env: {
			DB: bindings.d1({
				name: "decision-events",
				id: "68ade85e-6835-46a8-af6a-a9f2f2d966ae",
			}),
			SET_EVENTS: bindings.durableObject({
				worker: "decision-events",
				exportName: "DecisionSetEvents",
			}),
		},
		exports: {
			DecisionSetEvents: exports.durableObject({
				storage: "sqlite",
			}),
		},
	},
});
