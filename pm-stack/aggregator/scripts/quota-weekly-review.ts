#!/usr/bin/env bun
// The Stage-3 weekly forecast-accuracy review job (issue #12701,
// slice S7 of epic #12694; design
// doc/2026-11-22T22-35-00Z_design_quota-plane-stage-3 §7, D7).
//
//   bun run scripts/quota-weekly-review.ts --store-dir <dir> \
//     [--route-config <opencode.jsonc>] [--forecast <json>] \
//     [--shadow <json>] [--as-of <iso>] [--week-start <iso>] \
//     [--out-dir <dir>] [--report-out <md>] [--repo-root <dir>] \
//     [--gh <path>] [--base <branch>] [--open-prs] [--post]
//
// (Committed as the package.json script `review:quota-weekly`.)
//
// One run of the job: (1) builds S2's accuracy ledger + scored facts
// from the snapshot store and scores the week (the 7 days ending at
// --as-of, or the store's last reading) with src/quota-weekly-review.ts;
// (2) replays the §6 anomaly detectors over the store and keeps the
// week's findings; (3) when --route-config is supplied, runs S6's
// re-weighting generator over the ledger — the §5 fold — writing
// draft packs under --out-dir and, with --open-prs, opening the
// proposal PRs through quota-reweight-pr.ts's PR-only path; (4) with
// --post, posts the rendered report as a comment on epic #12694.
// Without --post the report goes to --report-out or stdout. The job
// never edits quota config in place: its only writes are its own
// outputs, proposal PRs (S6's mechanism), and the epic comment.
//
// The store is read through SnapshotStore.readAll — read-only; this
// script never writes to the store dir. --as-of defaults to the
// store's last reading, never the wall clock, so a run over a fixed
// store copy is reproducible.
//
// Argument contract (#3352): the whole command line is declared
// here — the value options and the two flags above, in `--opt
// value` or `--opt=value` form — and parsed strictly before any
// file is read or written. An unknown argument, a stray positional,
// or a present-but-valueless option is a usage error (exit 2)
// naming the offending argument.
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { SnapshotStore } from "../src/snapshots.ts";
import { buildForecastAccuracyLedger } from "../src/quota-forecast-accuracy.ts";
import { runAnomalyReplay } from "../src/quota-anomaly.ts";
import { buildReweightProposals, renderEvidencePack, type RouteWeightTable } from "../src/quota-reweight.ts";
import { nodeRunner, openProposalPr } from "../src/quota-reweight-pr.ts";
import { buildWeeklyReview, renderWeeklyReview } from "../src/quota-weekly-review.ts";
import type { QuotaForecastFile } from "../src/quota-forecast.ts";
import type { ShadowPlacementRecord } from "../src/quota-shadow-placement.ts";

const EPIC_REPO = "developing-today/code";
const EPIC_ISSUE = "12694";
const WEEK_MS = 7 * 24 * 3_600_000;

const VALUE_OPTIONS = [
  "--store-dir",
  "--route-config",
  "--forecast",
  "--shadow",
  "--as-of",
  "--week-start",
  "--out-dir",
  "--report-out",
  "--repo-root",
  "--gh",
  "--base",
] as const;
type ValueOption = (typeof VALUE_OPTIONS)[number];
const FLAG_OPTIONS = ["--open-prs", "--post"] as const;

function usageError(message: string): never {
  console.error(`quota-weekly-review: ${message}`);
  console.error(
    "usage: quota-weekly-review --store-dir <dir> [--route-config <opencode.jsonc>] [--forecast <json>] " +
      "[--shadow <json>] [--as-of <iso>] [--week-start <iso>] [--out-dir <dir>] [--report-out <md>] " +
      "[--repo-root <dir>] [--gh <path>] [--base <branch>] [--open-prs] [--post]",
  );
  process.exit(2);
}

function parseArgs(argv: string[]): {
  values: Record<ValueOption, string | undefined>;
  openPrs: boolean;
  post: boolean;
} {
  const values: Record<ValueOption, string | undefined> = {
    "--store-dir": undefined,
    "--route-config": undefined,
    "--forecast": undefined,
    "--shadow": undefined,
    "--as-of": undefined,
    "--week-start": undefined,
    "--out-dir": undefined,
    "--report-out": undefined,
    "--repo-root": undefined,
    "--gh": undefined,
    "--base": undefined,
  };
  let openPrs = false;
  let post = false;
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i] as string;
    const eq = arg.indexOf("=");
    const name = eq >= 0 ? arg.slice(0, eq) : arg;
    if ((FLAG_OPTIONS as readonly string[]).includes(name)) {
      if (eq >= 0) usageError(`flag '${name}' takes no value; refusing to run, no files read or written`);
      if (name === "--open-prs") openPrs = true;
      else post = true;
      continue;
    }
    if (!(VALUE_OPTIONS as readonly string[]).includes(name)) {
      usageError(`unknown argument '${arg}'; refusing to run, no files read or written`);
    }
    const opt = name as ValueOption;
    let value: string | undefined;
    if (eq >= 0) {
      value = arg.slice(eq + 1);
    } else {
      value = argv[i + 1];
      if (value !== undefined) i++;
    }
    if (value === undefined || value.trim() === "" || value.startsWith("--")) {
      usageError(`option '${opt}' requires a non-empty value; refusing to run, no files read or written`);
    }
    values[opt] = value;
  }
  return { values, openPrs, post };
}

// --- Route-config reading, mirrored from
// scripts/quota-reweight-proposals.ts (the S6 generator's script).
// The pair should be extracted to src/ when a third consumer
// appears; until then the mirror is deliberate and noted here so
// the two cannot drift unnoticed.

/** Strip line and block comments from JSONC without touching
 * string literals (the route config's comments are load-bearing
 * prose, but its strings carry URLs and model ids a naive strip
 * would corrupt). */
function stripJsoncComments(text: string): string {
  let out = "";
  let inString = false;
  let escaped = false;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i] as string;
    const next = text[i + 1] ?? "";
    if (inString) {
      out += ch;
      if (escaped) escaped = false;
      else if (ch === "\\") escaped = true;
      else if (ch === '"') inString = false;
      continue;
    }
    if (ch === '"') {
      inString = true;
      out += ch;
      continue;
    }
    if (ch === "/" && next === "/") {
      while (i < text.length && text[i] !== "\n") i++;
      out += "\n";
      continue;
    }
    if (ch === "/" && next === "*") {
      i += 2;
      while (i < text.length && !(text[i] === "*" && text[i + 1] === "/")) i++;
      i++;
      continue;
    }
    out += ch;
  }
  return dropTrailingCommas(out);
}

/** Remove trailing commas (a comma whose next non-whitespace
 * character closes an object or array) — JSONC permits them,
 * JSON.parse does not. String-aware, like the comment pass. */
function dropTrailingCommas(text: string): string {
  let out = "";
  let inString = false;
  let escaped = false;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i] as string;
    if (inString) {
      out += ch;
      if (escaped) escaped = false;
      else if (ch === "\\") escaped = true;
      else if (ch === '"') inString = false;
      continue;
    }
    if (ch === '"') {
      inString = true;
      out += ch;
      continue;
    }
    if (ch === ",") {
      let j = i + 1;
      while (j < text.length && /\s/.test(text[j] as string)) j++;
      if (text[j] === "}" || text[j] === "]") continue;
    }
    out += ch;
  }
  return out;
}

/** Extract every route carrying a selection + numeric weights map
 * from the parsed route config, and anchor each one's weights
 * line in the original (commented) text for exact find/replace. */
function extractRoutes(configText: string): RouteWeightTable[] {
  const parsed = JSON.parse(stripJsoncComments(configText)) as unknown;
  const routes: RouteWeightTable[] = [];
  const walk = (node: unknown): void => {
    if (Array.isArray(node)) {
      for (const item of node) walk(item);
      return;
    }
    if (node === null || typeof node !== "object") return;
    for (const [key, value] of Object.entries(node as Record<string, unknown>)) {
      if (value !== null && typeof value === "object" && !Array.isArray(value)) {
        const v = value as Record<string, unknown>;
        const weights = v.weights;
        if (
          typeof v.selection === "string" &&
          weights !== null &&
          typeof weights === "object" &&
          !Array.isArray(weights) &&
          Object.values(weights as Record<string, unknown>).every((n) => typeof n === "number")
        ) {
          routes.push({
            routeId: key,
            selection: v.selection,
            weights: weights as Record<string, number>,
            weightsLine: findWeightsLine(configText, key),
          });
        }
      }
      walk(value);
    }
  };
  walk(parsed);
  return routes;
}

/** The exact source line carrying a route's weights object, or
 * null when the object spans lines (the generator then refuses
 * to edit it rather than guess). */
function findWeightsLine(configText: string, routeId: string): string | null {
  const anchor = configText.indexOf(`"${routeId}"`);
  if (anchor < 0) return null;
  const wIdx = configText.indexOf('"weights"', anchor);
  if (wIdx < 0) return null;
  const lineStart = configText.lastIndexOf("\n", wIdx) + 1;
  const lineEnd = configText.indexOf("\n", wIdx);
  const line = configText.slice(lineStart, lineEnd < 0 ? undefined : lineEnd);
  const afterWeights = line.slice(line.indexOf('"weights"'));
  return afterWeights.includes("}") ? line : null;
}

// --- end of the mirrored route-config reading.

async function readJson<T>(path: string, what: string): Promise<T> {
  try {
    return JSON.parse(await readFile(path, "utf8")) as T;
  } catch (err) {
    console.error(`quota-weekly-review: could not read ${what} at ${path}: ${(err as Error).message}`);
    process.exit(1);
  }
}

function patchFor(file: string, find: string, replace: string): string {
  return [`--- a/${file}`, `+++ b/${file}`, `-${find}`, `+${replace}`, ""].join("\n");
}

async function main(): Promise<void> {
  const { values, openPrs, post } = parseArgs(process.argv.slice(2));
  const storeDir = values["--store-dir"];
  if (!storeDir) usageError("--store-dir is required; refusing to run, no files read or written");
  if (openPrs && !values["--route-config"]) {
    usageError("--open-prs requires --route-config (there are no proposals to open without the S6 fold)");
  }
  if (post && !values["--out-dir"] && !values["--report-out"]) {
    usageError("--post requires --out-dir or --report-out (the posted text must exist as a file first)");
  }

  const scriptDir = dirname(fileURLToPath(import.meta.url));
  const repoRoot = resolve(values["--repo-root"] ?? join(scriptDir, "..", "..", ".."));
  const ghPath = values["--gh"] ?? "/usr/bin/gh";
  const base = values["--base"] ?? "main";

  const records = await new SnapshotStore(storeDir).readAll();
  if (!records.length) {
    console.error(`quota-weekly-review: no snapshot records in ${storeDir} — nothing to review`);
    process.exit(1);
  }
  const stamps = records.map((r) => Date.parse(r.ts)).filter(Number.isFinite);
  const storeLastMs = Math.max(...stamps);

  let weekEndMs = storeLastMs;
  if (values["--as-of"] !== undefined) {
    weekEndMs = Date.parse(values["--as-of"]);
    if (!Number.isFinite(weekEndMs)) usageError(`--as-of '${values["--as-of"]}' is not a parseable instant`);
  }
  let weekStartMs = weekEndMs - WEEK_MS;
  if (values["--week-start"] !== undefined) {
    weekStartMs = Date.parse(values["--week-start"]);
    if (!Number.isFinite(weekStartMs))
      usageError(`--week-start '${values["--week-start"]}' is not a parseable instant`);
    if (weekStartMs >= weekEndMs)
      usageError("--week-start must be before the week's end (--as-of or the store's last reading)");
  }

  const { ledger, facts } = buildForecastAccuracyLedger(records, { asOfMs: weekEndMs });
  const { findings } = runAnomalyReplay(records, { asOfMs: weekEndMs });

  // The §5 fold (S6): only when the route config is supplied. Its
  // absence is a recorded state of the report, never a silent skip.
  let reweight: ReturnType<typeof buildReweightProposals> | null = null;
  const opened: { key: string; branch: string; url: string }[] = [];
  if (values["--route-config"]) {
    const configText = await readFile(values["--route-config"], "utf8");
    const routes = extractRoutes(configText);
    const forecast = values["--forecast"]
      ? await readJson<QuotaForecastFile>(values["--forecast"], "forecast file")
      : null;
    const shadow = values["--shadow"]
      ? await readJson<ShadowPlacementRecord>(values["--shadow"], "shadow record")
      : null;
    const derivePath = join(repoRoot, "pm-stack", "aggregator", "src", "derive.ts");
    const deriveText = await readFile(derivePath, "utf8");
    const nearMatches = [...deriveText.matchAll(/export const NEAR_PCT = (\d+);/g)];
    if (nearMatches.length !== 1) {
      console.error(
        `quota-weekly-review: expected exactly one 'export const NEAR_PCT = <n>;' in ${derivePath}, found ${nearMatches.length}; refusing to run`,
      );
      process.exit(1);
    }
    const nearPctCurrent = Number((nearMatches[0] as RegExpMatchArray)[1]);
    reweight = buildReweightProposals({ ledger, routes, forecast, shadow, nearPctCurrent });

    if (values["--out-dir"]) {
      const outDir = values["--out-dir"];
      await mkdir(outDir, { recursive: true });
      for (const p of reweight.proposals) {
        const dir = join(outDir, p.key.replace(/[^a-zA-Z0-9._-]/g, "-"));
        await mkdir(dir, { recursive: true });
        await writeFile(join(dir, "proposal.json"), `${JSON.stringify(p, null, 2)}\n`);
        await writeFile(join(dir, "proposal.md"), renderEvidencePack(p));
        await writeFile(join(dir, "change.patch"), patchFor(p.target.file, p.target.find, p.target.replace));
      }
    }
    if (openPrs) {
      const outDir = values["--out-dir"] ?? join(repoRoot, ".quota-weekly-review-out");
      await mkdir(outDir, { recursive: true });
      for (const p of reweight.proposals) {
        const res = await openProposalPr({
          runner: nodeRunner,
          ghPath,
          repoRoot,
          scratchDir: join(outDir, "scratch"),
          proposal: p,
          base,
        });
        opened.push({ key: p.key, ...res });
        console.log(`opened proposal PR for ${p.key}: ${res.url}`);
      }
    }
  }

  const generatedAt = new Date().toISOString();
  const review = buildWeeklyReview({
    ledger,
    facts,
    findings,
    weekStartMs,
    weekEndMs,
    generatedAt,
    reweight,
    openedPrs: opened,
  });
  const report = renderWeeklyReview(review);

  const reportPath =
    values["--report-out"] ?? (values["--out-dir"] ? join(values["--out-dir"], "weekly-review.md") : null);
  if (reportPath) {
    await mkdir(dirname(reportPath), { recursive: true });
    await writeFile(reportPath, report, "utf8");
  } else {
    console.log(report);
  }
  if (values["--out-dir"]) {
    await writeFile(join(values["--out-dir"], "weekly-review.json"), `${JSON.stringify(review, null, 2)}\n`, "utf8");
  }

  if (post) {
    // The one gh call this job makes, built literally here: a
    // comment carrying the report file on the Stage-3 epic.
    const res = await nodeRunner(
      ghPath,
      ["issue", "comment", EPIC_ISSUE, "--repo", EPIC_REPO, "--body-file", reportPath as string],
      { cwd: repoRoot },
    );
    if (res.code !== 0) {
      console.error(
        `quota-weekly-review: posting the report to #${EPIC_ISSUE} failed (gh exit ${res.code}): ${res.stderr.trim()}`,
      );
      process.exit(1);
    }
    console.log(`posted the weekly review to ${EPIC_REPO}#${EPIC_ISSUE}: ${res.stdout.trim()}`);
  }

  console.error(
    `quota-weekly-review: week ${review.weekStartIso} → ${review.weekEndIso}; ` +
      `${review.pools.reduce((s, p) => s + p.week.forecastsScored, 0)} forecasts scored, ` +
      `${review.findings.total} finding(s), ${review.proposals.length} proposal(s)` +
      `${review.noChangeFinding ? ", no-change finding recorded" : ""}${post ? ", report posted" : ""}`,
  );
}

await main();
