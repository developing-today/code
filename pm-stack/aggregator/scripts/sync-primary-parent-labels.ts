#!/usr/bin/env bun
// Create the `primary-parent/<slug>` label family from the current
// `epic/<slug>` family (design §3 of
// doc/2026-11-12T22-50-00Z_design_pm-board-hierarchy-genealogy).
//
// The label family is sparse on purpose: a label exists so the primary
// parent can be marked when the native sub-issue slot cannot carry it, so
// the registry mirrors the epics that exist today. Labels are the only
// primary-parent form queryable in GitHub, so they must exist before any
// backfill can apply them.
//
// Idempotent: existing primary-parent labels are reported and left alone,
// never re-coloured or re-described. `--dry-run` performs the read and
// prints what it would create without writing.
//
// Argument contract (#3352): the whole command line is declared here and
// parsed strictly before any subprocess, network, or write. Unknown flags,
// stray positionals, and `--opt=value` (vs `--opt value`) are usage
// errors (exit 2). `--help` prints this usage and exits 0.
//
// Usage:
//   bun run scripts/sync-primary-parent-labels.ts --repo <owner/name>
//     [--dry-run] [--help]
// Env: FLEET_BIN (default `fleet`).
import { execFile } from "node:child_process";

const USAGE =
  "sync-primary-parent-labels: usage: bun run scripts/sync-primary-parent-labels.ts " +
  "--repo <owner/name> [--dry-run] [--help]";

interface Options {
  repo: string;
  dryRun: boolean;
}

function usageError(message: string): never {
  console.error(`sync-primary-parent-labels: ${message}\n${USAGE}`);
  process.exit(2);
}

function parseArgs(argv: string[]): Options {
  let repo: string | null = null;
  let dryRun = false;
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i] as string;
    if (arg.includes("=") && arg.startsWith("--")) usageError(`use '--opt value', not '${arg}'`);
    switch (arg) {
      case "--repo": {
        const value = argv[i + 1];
        if (value === undefined || value.trim() === "" || value.startsWith("-")) {
          usageError("--repo requires a non-empty value");
        }
        repo = value.trim();
        i++;
        break;
      }
      case "--dry-run":
        dryRun = true;
        break;
      case "--help":
        console.log(USAGE);
        process.exit(0);
        break;
      default:
        usageError(`unknown argument '${arg}'`);
        break;
    }
  }
  if (repo === null) usageError("--repo is required");
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repo)) usageError(`--repo must be <owner/name> (got ${JSON.stringify(repo)})`);
  return { repo, dryRun };
}

const FLEET_BIN = process.env.FLEET_BIN ?? "fleet";

function runFleet(args: string[]): string {
  const result = Bun.spawnSync([FLEET_BIN, ...args], { stdout: "pipe", stderr: "pipe" });
  if (result.exitCode !== 0) {
    const stderr = new TextDecoder().decode(result.stderr).trim();
    throw new Error(`fleet ${args.slice(0, 2).join(" ")} failed: ${stderr || `exit ${result.exitCode}`}`);
  }
  return new TextDecoder().decode(result.stdout);
}

interface GhLabel {
  name?: string;
  color?: string;
}

/** GitHub caps label names at 50 characters; `primary-parent/` is 16, so a
 * 34-character slug is the longest that fits. */
const LABEL_NAME_MAX = 50;
const PRIMARY_PARENT_PREFIX = "primary-parent/";

export interface LabelPlan {
  slug: string;
  name: string;
  color: string;
  description: string;
}

export function planPrimaryParentLabels(labels: GhLabel[]): {
  plan: LabelPlan[];
  existing: string[];
  skippedTooLong: string[];
} {
  const names = labels.map((label) => label.name ?? "").filter(Boolean);
  const existing = new Set(names.filter((name) => name.startsWith(PRIMARY_PARENT_PREFIX)));
  const plan: LabelPlan[] = [];
  const skippedTooLong: string[] = [];
  const taken = new Set(names);
  for (const label of labels) {
    const name = label.name ?? "";
    if (!name.startsWith("epic/")) continue;
    const slug = name.slice("epic/".length).trim();
    if (!slug) continue;
    const target = `${PRIMARY_PARENT_PREFIX}${slug}`;
    if (target.length > LABEL_NAME_MAX) {
      skippedTooLong.push(target);
      continue;
    }
    if (existing.has(target) || taken.has(target)) continue;
    plan.push({
      slug,
      name: target,
      color: (label.color ?? "").replace(/^#/, "") || "5319e7",
      description: `Primary parent epic (design §3): marks epic/${slug} when the native sub-issue parent cannot carry it`,
    });
    taken.add(target);
  }
  return { plan, existing: [...existing].sort(), skippedTooLong };
}

function main(): void {
  const options = parseArgs(process.argv.slice(2));
  const labels = JSON.parse(runFleet(["gh", "label", "list", "--repo", options.repo, "--limit", "500", "--json", "name,color"])) as GhLabel[];
  const { plan, existing, skippedTooLong } = planPrimaryParentLabels(labels);
  const created: string[] = [];
  const failures: { name: string; error: string }[] = [];
  for (const entry of plan) {
    if (options.dryRun) {
      created.push(entry.name);
      continue;
    }
    try {
      runFleet([
        "gh",
        "label",
        "create",
        entry.name,
        "--repo",
        options.repo,
        "--color",
        entry.color,
        "--description",
        entry.description,
      ]);
      created.push(entry.name);
    } catch (error) {
      failures.push({ name: entry.name, error: (error as Error).message });
    }
  }
  console.log(
    JSON.stringify(
      {
        repo: options.repo,
        dryRun: options.dryRun,
        existingPrimaryParentLabels: existing.length,
        created: created.sort(),
        failed: failures,
        skippedTooLong,
      },
      null,
      2,
    ),
  );
  if (failures.length > 0) process.exit(1);
}

if (import.meta.main) main();
