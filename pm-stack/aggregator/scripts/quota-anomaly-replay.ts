#!/usr/bin/env bun
// Replay the Stage-3 anomaly detectors over the aggregator
// snapshot store (#12699, slice S5 of epic #12694; design
// doc/2026-11-22T22-35-00Z_design_quota-plane-stage-3 §6, D6).
//
//   bun run scripts/quota-anomaly-replay.ts --store-dir <dir> \
//     [--report-out <md>] [--findings-out <jsonl>] [--as-of <iso>]
//
// (Committed as the package.json script `replay:quota-anomaly`.)
//
// The store is read through SnapshotStore.readAll — read-only; this
// script never writes to the store dir. The replay runs the same
// machines the gatherer tick runs (src/quota-anomaly.ts) over the
// store's stream in order and reports, per detector: every finding
// (the historical incidents it fires on), the per-series evaluation
// rollup (fired / silent / unevaluable / baseline-established — an
// unevaluable series is named, never counted as silence), and the
// longest finding-free span of >= 7 days inside the store span
// (the replayed clean week the acceptance asks each detector for).
// It also renders, for the first finding, the two records a live
// tick files for it — the evidence-baseline diff record (the
// history JSONL line) and the alerts-ledger incident input — the
// replay-injected demonstration of the finding's landing surfaces.
//
// Replay fidelity, stated plainly: burn-spike and the plan-change
// limit leg replay at full fidelity (the store carries their whole
// input). Schema drift replays at store fidelity — per-service
// per-day (metric, unit) field sets, removals sustained over 3
// active days because the store appends only on movement. The
// unparseable leg has no store trace (the store records no parse
// outcomes); it is covered by the live tick path and the unit
// fixtures, not by this replay.
//
// --as-of defaults to the store's last reading, never the wall
// clock, so a run over a fixed store copy is reproducible.
//
// Argument contract (#3352): the whole command line is declared
// here — the four value options above, in `--opt value` or
// `--opt=value` form — and parsed strictly before any file is read
// or written. An unknown argument, a stray positional, or a
// present-but-valueless option is a usage error (exit 2) naming
// the offending argument.
import { writeFile } from "node:fs/promises";
import { SnapshotStore } from "../src/snapshots.ts";
import {
  anomalyIncidentInput,
  runAnomalyReplay,
  type AnomalyDetectorId,
  type AnomalyFinding,
  type DetectorEvaluation,
} from "../src/quota-anomaly.ts";

const VALUE_OPTIONS = ["--store-dir", "--report-out", "--findings-out", "--as-of"] as const;
type ValueOption = (typeof VALUE_OPTIONS)[number];

function usageError(message: string): never {
  console.error(`quota-anomaly-replay: ${message}`);
  console.error(
    "usage: quota-anomaly-replay --store-dir <dir> [--report-out <md>] [--findings-out <jsonl>] [--as-of <iso>]",
  );
  process.exit(2);
}

function parseArgs(argv: string[]): Record<ValueOption, string | undefined> {
  const out: Record<ValueOption, string | undefined> = {
    "--store-dir": undefined,
    "--report-out": undefined,
    "--findings-out": undefined,
    "--as-of": undefined,
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i] as string;
    const eq = arg.indexOf("=");
    const name = (eq >= 0 ? arg.slice(0, eq) : arg) as ValueOption;
    if (!VALUE_OPTIONS.includes(name)) {
      usageError(`unknown argument '${arg}'; refusing to run, no files read or written`);
    }
    let value: string | undefined;
    if (eq >= 0) {
      value = arg.slice(eq + 1);
    } else {
      value = argv[i + 1];
      if (value !== undefined) i++;
    }
    if (value === undefined || value.trim() === "" || value.startsWith("--")) {
      usageError(`option '${name}' requires a non-empty value; refusing to run, no files read or written`);
    }
    out[name] = value;
  }
  return out;
}

const DAY_MS = 86_400_000;

/** The longest span inside [fromMs, toMs] containing no finding of
 * this detector; null when no >= 7-day span exists. */
function longestCleanSpan(
  findings: readonly AnomalyFinding[],
  detector: AnomalyDetectorId,
  fromMs: number,
  toMs: number,
): { startIso: string; endIso: string; days: number } | null {
  const cuts = findings
    .filter((f) => f.detector === detector)
    .map((f) => Date.parse(f.at))
    .filter(Number.isFinite)
    .sort((a, b) => a - b);
  let best: { start: number; end: number } | null = null;
  let prev = fromMs;
  for (const cut of [...cuts, toMs]) {
    if (!best || cut - prev > best.end - best.start) best = { start: prev, end: cut };
    prev = Math.max(prev, cut);
  }
  if (!best || best.end - best.start < 7 * DAY_MS) return null;
  return {
    startIso: new Date(best.start).toISOString(),
    endIso: new Date(best.end).toISOString(),
    days: Math.round(((best.end - best.start) / DAY_MS) * 10) / 10,
  };
}

function renderReport(args: {
  storeDir: string;
  records: number;
  fromIso: string | null;
  toIso: string | null;
  fromMs: number;
  toMs: number;
  findings: AnomalyFinding[];
  evaluations: DetectorEvaluation[];
}): string {
  const lines: string[] = [];
  lines.push("# Quota anomaly detector replay — Stage-3 S5 (#12699)");
  lines.push("");
  lines.push(`- Store: ${args.records} records, ${args.fromIso ?? "?"} → ${args.toIso ?? "?"} (dir: ${args.storeDir})`);
  lines.push(
    "- Method: the live detectors' own machines (src/quota-anomaly.ts) replayed over the snapshot store in stream order. Burn-spike and the plan-change limit leg replay at full fidelity; schema drift replays at store fidelity (per-service per-day field sets, 3-active-day removal sustain — the store appends only on movement); the unparseable leg has no store trace and is covered by the live path + unit fixtures, not this replay.",
  );
  lines.push("");
  const detectors: AnomalyDetectorId[] = ["burn-spike", "schema-drift", "plan-change"];
  for (const detector of detectors) {
    const own = args.findings.filter((f) => f.detector === detector);
    const evals = args.evaluations.filter((e) => e.detector === detector);
    const count = (s: string): number => evals.filter((e) => e.state === s).length;
    lines.push(`## ${detector} — ${own.length} finding(s)`);
    lines.push("");
    lines.push(
      `Evaluations: ${evals.length} series — fired ${count("fired")}, silent ${count("silent")}, unevaluable ${count("unevaluable")}, baseline-established ${count("baseline-established")}.`,
    );
    const clean = longestCleanSpan(args.findings, detector, args.fromMs, args.toMs);
    lines.push(
      clean
        ? `Clean week: silent across ${clean.startIso} → ${clean.endIso} (${clean.days} days, the longest finding-free span in the store).`
        : "Clean week: NO finding-free span of >= 7 days exists in the store for this detector — the period was change-dense for this signal throughout (stated, not rounded away).",
    );
    lines.push("");
    for (const f of own) {
      lines.push(`- **${f.at}** ${f.serviceId}${f.window ? ` ${f.window}` : ""} — ${f.title}`);
      lines.push(`  - baseline: ${f.baseline}`);
      lines.push(`  - observed: ${f.observed}`);
    }
    if (!own.length) lines.push("- (no findings over the store span)");
    lines.push("");
    const unevaluable = evals.filter((e) => e.state === "unevaluable");
    if (unevaluable.length) {
      lines.push("Unevaluable series (named, never counted as silence):");
      for (const e of unevaluable) {
        lines.push(`- ${e.serviceId}${e.window ? ` ${e.window}` : ""}: ${e.reason ?? ""}`);
      }
      lines.push("");
    }
  }
  const first = args.findings[0];
  if (first) {
    lines.push("## Landing demonstration (replay-injected)");
    lines.push("");
    lines.push(
      'For the first finding above, the two records a live gatherer tick files — the evidence-baseline diff record (one history JSONL line) and the alerts-ledger incident input (AlertsStore.postIncident, source "quota-anomaly"):',
    );
    lines.push("");
    lines.push("```json");
    lines.push(JSON.stringify({ kind: "quota-anomaly-finding", version: 1, ...first }));
    lines.push(JSON.stringify(anomalyIncidentInput(first)));
    lines.push("```");
    lines.push("");
  }
  return lines.join("\n");
}

async function main(): Promise<void> {
  const args = parseArgs(process.argv.slice(2));
  const storeDir = args["--store-dir"];
  if (!storeDir) usageError("--store-dir is required; refusing to run, no files read or written");
  let asOfMs: number | undefined;
  if (args["--as-of"] !== undefined) {
    asOfMs = Date.parse(args["--as-of"]);
    if (!Number.isFinite(asOfMs)) usageError(`--as-of '${args["--as-of"]}' is not a parseable instant`);
  }
  const records = await new SnapshotStore(storeDir as string).readAll();
  const stamps = records.map((r) => Date.parse(r.ts)).filter(Number.isFinite);
  const fromMs = stamps.length ? Math.min(...stamps) : 0;
  const toMs = stamps.length ? Math.max(...stamps) : 0;
  const { findings, evaluations } = runAnomalyReplay(records, { asOfMs });
  const report = renderReport({
    storeDir: storeDir as string,
    records: records.length,
    fromIso: stamps.length ? new Date(fromMs).toISOString() : null,
    toIso: stamps.length ? new Date(toMs).toISOString() : null,
    fromMs,
    toMs,
    findings,
    evaluations,
  });
  if (args["--report-out"]) {
    await writeFile(args["--report-out"], report, "utf8");
    console.log(`quota-anomaly-replay: report -> ${args["--report-out"]} (${findings.length} findings)`);
  } else {
    console.log(report);
  }
  if (args["--findings-out"]) {
    const jsonl = findings.map((f) => JSON.stringify({ kind: "quota-anomaly-finding", version: 1, ...f })).join("\n");
    await writeFile(args["--findings-out"], jsonl ? `${jsonl}\n` : "", "utf8");
    console.log(`quota-anomaly-replay: findings -> ${args["--findings-out"]}`);
  }
}

await main();
