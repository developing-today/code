#!/usr/bin/env node
// Fixture producer stub for the quota watchdog (#6032 acceptance).
//
// Stands in for a real browser-dependent producer: each invocation
// reads a scenario file and writes the producer document
// (`latest.json`, atomically) plus one `history.jsonl` line, in the
// exact shape the watchdog observes. Tests and live demos drive the
// watchdog through its acceptance cases by rewriting the scenario
// between ticks — no browser, no network, no secrets.
//
// Scenario file (JSON, path from FIXTURE_SCENARIO or argv[2]):
//   { "status": "ok" }                                  healthy tick
//   { "status": "session-expired", "detail": "..." }    pre-classified failure
//   { "status": "rate-limited" }                        rung-0 cooldown case
//   { "status": "error", "detail": "human-only challenge: 2fa required" }
//   { "status": "ok", "measuredAt": "<old ISO>" }       stale-read case
//   { "missing": true }                                 writes NO document
//
// Environment:
//   FIXTURE_STATE_DIR  where latest.json/history.jsonl land (required)
//   FIXTURE_SCENARIO   scenario file path (or argv[2])

import { mkdir, readFile, rename, writeFile, appendFile, rm } from "node:fs/promises";
import { join } from "node:path";

const stateDir = process.env.FIXTURE_STATE_DIR;
if (!stateDir) {
  console.error("fixture-producer: FIXTURE_STATE_DIR is required");
  process.exit(2);
}
const scenarioPath = process.argv[2] ?? process.env.FIXTURE_SCENARIO;
if (!scenarioPath) {
  console.error("fixture-producer: scenario path required (argv[2] or FIXTURE_SCENARIO)");
  process.exit(2);
}

const scenario = JSON.parse(await readFile(scenarioPath, "utf8"));
await mkdir(stateDir, { recursive: true });
const latestPath = join(stateDir, "latest.json");

if (scenario.missing === true) {
  await rm(latestPath, { force: true });
  process.exit(0);
}

const now = new Date().toISOString();
const doc = {
  service: "fixture",
  source: "file-export/fixture",
  status: scenario.status ?? "ok",
  measuredAt: scenario.measuredAt ?? now,
  ...(scenario.detail ? { detail: scenario.detail } : {}),
  ...(scenario.failureClass ? { failureClass: scenario.failureClass } : {}),
  windows: scenario.status === "ok" || !scenario.status ? [{ name: "fixture", usedPct: 0, resetAt: null }] : [],
};

// Atomic publish, the producer discipline the watchdog relies on.
const tmp = `${latestPath}.tmp-${process.pid}`;
await writeFile(tmp, `${JSON.stringify(doc, null, 2)}\n`);
await rename(tmp, latestPath);
await appendFile(join(stateDir, "history.jsonl"), `${JSON.stringify({ at: now, status: doc.status })}\n`);
