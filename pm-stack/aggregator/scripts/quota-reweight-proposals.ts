#!/usr/bin/env bun
// Generate Stage-3 re-weighting proposals from the forecast
// accuracy ledger (issue #12700, slice S6 of epic #12694; design
// doc/2026-11-22T22-35-00Z_design_quota-plane-stage-3 §5, D5).
//
//   bun run scripts/quota-reweight-proposals.ts --ledger <json> \
//     --route-config <opencode.jsonc> --out-dir <dir> \
//     [--forecast <json>] [--shadow <json>] [--repo-root <dir>] \
//     [--gh <path>] [--base <branch>] [--open-prs]
//
// (Committed as the package.json script `proposals:quota-reweight`.)
//
// Draft mode (default) writes, per proposal, a directory holding
// proposal.json, the evidence pack (proposal.md — the exact text
// the PR body carries), and change.patch; plus a run report.md
// listing proposals AND evidence gaps. Nothing touches git or gh
// in draft mode. With --open-prs, each proposal additionally goes
// through quota-reweight-pr.ts's PR-only path: a quota-proposal/…
// branch and a `gh pr create` — never a merge, never a push to
// main (the allowlist there is the enforcement; see that module).
//
// Inputs: --ledger is S2's accuracy ledger JSON (the
// `backtest:quota-forecast --ledger-out` product today; the same
// shape scored from live forecast rows later — the generator's
// contract is the ledger shape, not its provenance). --forecast
// and --shadow are the S1/S3 products from the aggregator state
// dir when they exist; both are optional evidence-pack inputs,
// and their absence is reported in the pack, never papered over.
//
// Argument contract (#3352): the whole command line is declared
// here — the value options and the one flag above, in `--opt
// value` or `--opt=value` form — and parsed strictly before any
// file is read or written. An unknown argument, a stray
// positional, or a present-but-valueless option is a usage error
// (exit 2) naming the offending argument.
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { ForecastAccuracyLedger } from "../src/quota-forecast-accuracy.ts";
import type { QuotaForecastFile } from "../src/quota-forecast.ts";
import type { ShadowPlacementRecord } from "../src/quota-shadow-placement.ts";
import {
  buildReweightProposals,
  renderEvidencePack,
  renderRunReport,
  type RouteWeightTable,
} from "../src/quota-reweight.ts";
import { nodeRunner, openProposalPr } from "../src/quota-reweight-pr.ts";

const VALUE_OPTIONS = [
  "--ledger",
  "--route-config",
  "--forecast",
  "--shadow",
  "--out-dir",
  "--repo-root",
  "--gh",
  "--base",
] as const;
type ValueOption = (typeof VALUE_OPTIONS)[number];
const FLAG_OPTIONS = ["--open-prs"] as const;

function usageError(message: string): never {
  console.error(`quota-reweight-proposals: ${message}`);
  console.error(
    "usage: quota-reweight-proposals --ledger <json> --route-config <opencode.jsonc> --out-dir <dir> " +
      "[--forecast <json>] [--shadow <json>] [--repo-root <dir>] [--gh <path>] [--base <branch>] [--open-prs]",
  );
  process.exit(2);
}

function parseArgs(argv: string[]): { values: Record<ValueOption, string | undefined>; openPrs: boolean } {
  const values: Record<ValueOption, string | undefined> = {
    "--ledger": undefined,
    "--route-config": undefined,
    "--forecast": undefined,
    "--shadow": undefined,
    "--out-dir": undefined,
    "--repo-root": undefined,
    "--gh": undefined,
    "--base": undefined,
  };
  let openPrs = false;
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i] as string;
    const eq = arg.indexOf("=");
    const name = eq >= 0 ? arg.slice(0, eq) : arg;
    if ((FLAG_OPTIONS as readonly string[]).includes(name)) {
      if (eq >= 0) usageError(`flag '${name}' takes no value; refusing to run, no files read or written`);
      openPrs = true;
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
  return { values, openPrs };
}

/** Strip // and /* *\/ comments from JSONC without touching string
 * literals (the route config's comments are load-bearing prose,
 * but its strings carry URLs and model ids a naive strip would
 * corrupt). */
export function stripJsoncComments(text: string): string {
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
export function extractRoutes(configText: string): RouteWeightTable[] {
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

async function readJson<T>(path: string, what: string): Promise<T> {
  try {
    return JSON.parse(await readFile(path, "utf8")) as T;
  } catch (err) {
    console.error(`quota-reweight-proposals: could not read ${what} at ${path}: ${(err as Error).message}`);
    process.exit(1);
  }
}

function patchFor(file: string, find: string, replace: string): string {
  return [`--- a/${file}`, `+++ b/${file}`, `-${find}`, `+${replace}`, ""].join("\n");
}

async function main(): Promise<void> {
  const { values, openPrs } = parseArgs(process.argv.slice(2));
  const ledgerPath = values["--ledger"];
  const routeConfigPath = values["--route-config"];
  const outDir = values["--out-dir"];
  if (!ledgerPath) usageError("--ledger is required; refusing to run, no files read or written");
  if (!routeConfigPath) usageError("--route-config is required; refusing to run, no files read or written");
  if (!outDir) usageError("--out-dir is required; refusing to run, no files read or written");

  const scriptDir = dirname(fileURLToPath(import.meta.url));
  const repoRoot = resolve(values["--repo-root"] ?? join(scriptDir, "..", "..", ".."));
  const ghPath = values["--gh"] ?? "/usr/bin/gh";
  const base = values["--base"] ?? "main";

  const ledger = await readJson<ForecastAccuracyLedger>(ledgerPath, "accuracy ledger");
  if (ledger?.kind !== "quota-forecast-accuracy" || !Array.isArray(ledger.pools)) {
    console.error(`quota-reweight-proposals: ${ledgerPath} is not a quota-forecast-accuracy ledger`);
    process.exit(1);
  }
  const forecast = values["--forecast"]
    ? await readJson<QuotaForecastFile>(values["--forecast"], "forecast file")
    : null;
  const shadow = values["--shadow"]
    ? await readJson<ShadowPlacementRecord>(values["--shadow"], "shadow record")
    : null;
  const configText = await readFile(routeConfigPath, "utf8");
  const routes = extractRoutes(configText);

  const derivePath = join(repoRoot, "pm-stack", "aggregator", "src", "derive.ts");
  const deriveText = await readFile(derivePath, "utf8");
  const nearMatches = [...deriveText.matchAll(/export const NEAR_PCT = (\d+);/g)];
  if (nearMatches.length !== 1) {
    console.error(
      `quota-reweight-proposals: expected exactly one 'export const NEAR_PCT = <n>;' in ${derivePath}, found ${nearMatches.length}; refusing to run`,
    );
    process.exit(1);
  }
  const nearPctCurrent = Number((nearMatches[0] as RegExpMatchArray)[1]);

  const run = buildReweightProposals({ ledger, routes, forecast, shadow, nearPctCurrent });
  const generatedAt = new Date().toISOString();

  await mkdir(outDir, { recursive: true });
  for (const p of run.proposals) {
    const dir = join(outDir, p.key.replace(/[^a-zA-Z0-9._-]/g, "-"));
    await mkdir(dir, { recursive: true });
    await writeFile(join(dir, "proposal.json"), `${JSON.stringify(p, null, 2)}\n`);
    await writeFile(join(dir, "proposal.md"), renderEvidencePack(p));
    await writeFile(join(dir, "change.patch"), patchFor(p.target.file, p.target.find, p.target.replace));
  }

  const opened: { key: string; branch: string; url: string }[] = [];
  if (openPrs) {
    for (const p of run.proposals) {
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
    if (opened.length) await writeFile(join(outDir, "prs.json"), `${JSON.stringify(opened, null, 2)}\n`);
  }

  let report = renderRunReport(run, generatedAt);
  if (opened.length) {
    report += `\n## Proposal PRs opened\n\n${opened.map((o) => `- \`${o.key}\`: ${o.url}`).join("\n")}\n`;
  }
  await writeFile(join(outDir, "report.md"), report);
  console.log(
    `quota-reweight-proposals: ${run.proposals.length} proposal(s), ${run.gaps.length} evidence gap(s); ` +
      `drafts in ${outDir}${openPrs ? `; ${opened.length} PR(s) opened` : ""}`,
  );
}

await main();
