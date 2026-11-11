#!/usr/bin/env bun
// Capture one weekly forecast snapshot into the backtest store
// (#3986). The weekly job is this command on a timer:
//
//   bun run scripts/forecast-snapshot.ts \
//     --forecasts forecasts.json --out data/forecast-snapshots.jsonl \
//     --watermark item-history=<cursor> [--cutoff <iso>]
//
// --forecasts is a JSON file holding ForecastContract[] (or an object
// with a `forecasts` array) as served by GET /api/forecasts.json. The
// snapshot freezes the epic-completion forecasts with their model
// version, the cutoff, and the source watermarks that name exactly
// what source state they were derived from.
import { readFile } from "node:fs/promises";
import { appendSnapshot, createSnapshot } from "../src/backtest.ts";
import type { ForecastContract } from "../src/forecast-contract.ts";

function arg(name: string): string | undefined {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

const forecastsPath = arg("forecasts");
const out = arg("out");
if (!forecastsPath || !out) {
  console.error("usage: forecast-snapshot.ts --forecasts <json> --out <jsonl> [--cutoff <iso>] [--watermark k=v ...]");
  process.exit(2);
}
const parsed = JSON.parse(await readFile(forecastsPath, "utf8")) as ForecastContract[] | { forecasts: ForecastContract[] };
const forecasts = Array.isArray(parsed) ? parsed : parsed.forecasts;
const sourceWatermarks: Record<string, string> = {};
for (let i = 0; i < process.argv.length; i++) {
  if (process.argv[i] === "--watermark") {
    const [k, ...v] = (process.argv[i + 1] ?? "").split("=");
    if (k) sourceWatermarks[k] = v.join("=");
  }
}
const snapshot = createSnapshot({
  forecasts,
  capturedAt: new Date().toISOString(),
  sourceWatermarks,
  cutoff: arg("cutoff"),
});
await appendSnapshot(out, snapshot);
console.log(`snapshot: model=${snapshot.modelVersion} cutoff=${snapshot.cutoff} forecasts=${snapshot.forecasts.length} -> ${out}`);
