// Contract: bun run scripts/bench-activity-dedupe.ts [--module ABSOLUTE_PATH]
// Reject unknown/missing arguments before loading code or running measurements.
// --module selects a baseline copy of activity.ts; defaults to the current code.
import { isAbsolute } from "node:path";
import { pathToFileURL } from "node:url";
import type { ActivityDelivery, IngestionFile } from "../src/activity.ts";

const args = process.argv.slice(2);
if (args.length && (args.length !== 2 || args[0] !== "--module" || !isAbsolute(args[1]!))) {
  throw new Error("usage: bench-activity-dedupe.ts [--module ABSOLUTE_PATH]");
}
const moduleUrl = args.length ? pathToFileURL(args[1]!) : new URL("../src/activity.ts", import.meta.url);
const { IngestionStore } = (await import(moduleUrl.href)) as typeof import("../src/activity.ts");
const at = "2026-11-25T00:00:00.000Z";
const ids = Array.from({ length: 2000 }, (_, i) => `github:delivery:${i}`);
const events = { record: async (): Promise<never> => { throw new Error("replay must not append"); } };

// Measure the real public ingest boundary including lookup-index construction,
// checkpoint updates and persist dispatch. No-op remote save isolates CPU from IO.
for (const size of [1, 100, 2000]) {
  const state: IngestionFile = { version: 1, checkpoints: {}, seen: { github: [...ids] }, gaps: [] };
  const store = new IngestionStore("unused", { load: async () => state, save: async () => {} });
  const batch: ActivityDelivery[] = Array.from({ length: size }, (_, i) => ({
    source: "github", sourceId: ids[(i * 791 + 1999) % ids.length]!, at,
    kind: "github-event", ref: "developing-today/code", summary: "replay",
  }));
  for (let i = 0; i < 10; i++) await store.ingest(events, batch);
  const samples: number[] = [];
  for (let sample = 0; sample < 5; sample++) {
    const start = performance.now();
    for (let i = 0; i < 100; i++) {
      const result = await store.ingest(events, batch);
      if (result.duplicates !== size || result.ingested !== 0) throw new Error("replay oracle failed");
    }
    samples.push(performance.now() - start);
  }
  samples.sort((a, b) => a - b);
  console.log(JSON.stringify({ retainedIds: ids.length, batchSize: size, iterations: 100, medianMs: samples[2] }));
}
