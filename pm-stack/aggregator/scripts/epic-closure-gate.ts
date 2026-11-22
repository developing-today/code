#!/usr/bin/env bun
// Usage: bun run scripts/epic-closure-gate.ts --root owner/repo#123 [--root owner/repo#456]
// Reads complete native sub-issue trees from GitHub and prints per-epic blockers.

import { deriveEpicClosureGateFacts, evaluateEpicClosureGate } from "../src/epics.ts";
import { fetchNativeSubissueTree } from "../src/epic-subtree.ts";
import type { RefKey } from "../src/model.ts";

function parseArgs(args: string[]): RefKey[] {
  const roots: RefKey[] = [];
  for (let i = 0; i < args.length; i += 1) {
    if (args[i] !== "--root" || !args[i + 1])
      throw new Error("usage: epic-closure-gate.ts --root owner/repo#123 [--root owner/repo#456]");
    const root = args[++i]!;
    if (!/^[^/#]+\/[^/#]+#[1-9][0-9]*$/.test(root))
      throw new Error(`invalid --root ref: ${root}`);
    roots.push(root as RefKey);
  }
  if (roots.length === 0) throw new Error("at least one --root is required");
  if (new Set(roots).size !== roots.length) throw new Error("duplicate --root refs are not allowed");
  return roots;
}

function tokenFromEnvironmentOrGh(): string {
  const direct = process.env.GH_TOKEN || process.env.GITHUB_TOKEN;
  if (direct) return direct;
  const result = Bun.spawnSync(["gh", "auth", "token"], { stdout: "pipe", stderr: "pipe" });
  if (result.exitCode !== 0) throw new Error("no GitHub token; set GH_TOKEN or authenticate gh");
  const token = result.stdout.toString().trim();
  if (!token) throw new Error("gh auth token returned an empty token");
  return token;
}

const CONDITION_TEXT = {
  source_incomplete: "complete native subtree facts unavailable",
  designs_incomplete: "subtree has unfinished designs not explicitly Drew-gated",
  qualifying_open_descendants: "open high/critical bugs or follow-ups remain",
  unfinished_children_without_reparent_destination: "unfinished direct children lack a re-parent destination",
} as const;

async function main(): Promise<void> {
  // Validate the whole command line before credentials, network, or filesystem access.
  const roots = parseArgs(process.argv.slice(2));
  const token = tokenFromEnvironmentOrGh();
  const reports = [];
  for (const root of roots) {
    const descendants = await fetchNativeSubissueTree(root, token);
    const facts = deriveEpicClosureGateFacts(root, descendants);
    const gate = evaluateEpicClosureGate(facts);
    reports.push({
      root,
      descendants: descendants.length,
      factsComplete: facts.factsComplete,
      closable: gate.closable,
      blockers: gate.blockers.map((blocker) => ({
        condition: blocker.condition,
        description: CONDITION_TEXT[blocker.condition],
        refs: blocker.refs,
      })),
    });
  }
  console.log(JSON.stringify({ generatedAt: new Date().toISOString(), reports }, null, 2));
}

main().catch((error: unknown) => {
  console.error(`epic-closure-gate: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
});
