// Phase-1 new-issue intake poll for #1480: list issues created since the
// persisted watermark across the four repos, so the hourly orchestration
// pass can assess, label, and comment on every new issue in the same pass.
//
// Usage:
//   bun run scripts/gh-intake-poll.ts poll [--state <file>] [--since <iso>] [--repos a/b,c/d]
//   bun run scripts/gh-intake-poll.ts advance --to <iso> [--state <file>]
//
// `poll` is read-only: it never touches the watermark. The pass advances
// the watermark with `advance` only after the pass succeeds (assess +
// label + comment done), so a failed pass re-polls the same window.
// `advance` moves forward only and refuses invalid timestamps.
//
// State file: JSON `{"watermark": "<iso>"}`. Resolved from --state, then
// $PM_INTAKE_STATE_FILE, then ./intake-state.json. A missing state file
// on `poll` means "no watermark yet": the caller must pass --since to
// bootstrap explicitly (no silent epoch). Reads go through the gh CLI only.
import { readFile, writeFile } from "node:fs/promises";
import { INTAKE_REPOS, advanceWatermark, loadState, saveState, selectNewIssues, type IntakeItem } from "../src/intake.ts";

function usageError(message: string): never {
  console.error(`gh-intake-poll: ${message}`);
  console.error("usage: gh-intake-poll.ts poll [--state <file>] [--since <iso>] [--repos a/b,c/d]");
  console.error("       gh-intake-poll.ts advance --to <iso> [--state <file>]");
  process.exit(2);
}

function flagValue(args: string[], name: string): string | undefined {
  const idx = args.indexOf(name);
  if (idx < 0) return undefined;
  const value = args[idx + 1];
  if (value == null || value.startsWith("--")) usageError(`${name} needs a value`);
  return value;
}

function ghApi(path: string): unknown {
  const proc = Bun.spawnSync(["gh", "api", path], { stdout: "pipe", stderr: "pipe" });
  if (proc.exitCode !== 0) {
    throw new Error(`gh api ${path} failed (${proc.exitCode}): ${new TextDecoder().decode(proc.stderr).slice(0, 400)}`);
  }
  return JSON.parse(new TextDecoder().decode(proc.stdout));
}

function resolveStateFile(args: string[]): string {
  return flagValue(args, "--state") ?? process.env.PM_INTAKE_STATE_FILE ?? "intake-state.json";
}

async function readWatermark(stateFile: string, sinceOverride: string | undefined): Promise<string> {
  if (sinceOverride) {
    if (!Number.isFinite(Date.parse(sinceOverride))) usageError(`--since is not a valid timestamp: ${sinceOverride}`);
    return sinceOverride;
  }
  let text: string;
  try {
    text = await readFile(stateFile, "utf8");
  } catch {
    usageError(`no state file at ${stateFile} — pass --since <iso> to bootstrap explicitly`);
  }
  try {
    return loadState(text!).watermark;
  } catch (err) {
    usageError(`bad state file at ${stateFile}: ${(err as Error).message}`);
  }
}

/** Issues for one repo, newest first; stops paging once past the watermark. */
function pollRepo(repo: string, watermark: string): IntakeItem[] {
  const mark = Date.parse(watermark);
  const raw: Record<string, unknown>[] = [];
  for (let page = 1; page <= 10; page++) {
    const batch = ghApi(
      `repos/${repo}/issues?state=all&sort=created&direction=desc&per_page=100&page=${page}`,
    ) as Record<string, unknown>[];
    if (!Array.isArray(batch) || batch.length === 0) break;
    raw.push(...batch);
    const oldest = batch[batch.length - 1]?.created_at;
    if (typeof oldest === "string" && Number.isFinite(Date.parse(oldest)) && Date.parse(oldest) <= mark) break;
    if (batch.length < 100) break;
  }
  return selectNewIssues(repo, raw, watermark);
}

async function cmdPoll(args: string[]): Promise<void> {
  const stateFile = resolveStateFile(args);
  const since = flagValue(args, "--since");
  const reposCsv = flagValue(args, "--repos");
  const repos = reposCsv ? reposCsv.split(",").map((s) => s.trim()).filter(Boolean) : [...INTAKE_REPOS];
  for (const repo of repos) {
    if (!(INTAKE_REPOS as readonly string[]).includes(repo)) usageError(`unknown repo (not one of the four): ${repo}`);
  }
  const watermark = await readWatermark(stateFile, since);
  const items: IntakeItem[] = [];
  const errors: { repo: string; error: string }[] = [];
  for (const repo of repos) {
    try {
      items.push(...pollRepo(repo, watermark));
    } catch (err) {
      errors.push({ repo, error: (err as Error).message });
    }
  }
  items.sort((a, b) => (a.createdAt < b.createdAt ? -1 : a.createdAt > b.createdAt ? 1 : 0));
  console.log(JSON.stringify({ watermark, repos, count: items.length, items, errors }, null, 2));
  if (errors.length > 0) process.exit(1);
}

async function cmdAdvance(args: string[]): Promise<void> {
  const to = flagValue(args, "--to") ?? usageError("advance needs --to <iso>");
  const stateFile = resolveStateFile(args);
  let text: string;
  try {
    text = await readFile(stateFile, "utf8");
  } catch {
    usageError(`no state file at ${stateFile} — cannot advance without a current watermark`);
  }
  let next: { watermark: string };
  try {
    next = advanceWatermark(loadState(text!), to);
  } catch (err) {
    usageError((err as Error).message);
  }
  await writeFile(stateFile, saveState(next!));
  console.log(JSON.stringify(next));
}

async function main(): Promise<void> {
  const [cmd, ...rest] = process.argv.slice(2);
  if (rest.some((a) => a === "--help" || a === "-h")) {
    console.log("gh-intake-poll: phase-1 new-issue intake poll for #1480 (poll is read-only; advance is forward-only)");
    return;
  }
  if (cmd === "poll") return cmdPoll(rest);
  if (cmd === "advance") return cmdAdvance(rest);
  usageError(cmd == null ? "missing subcommand (poll | advance)" : `unknown subcommand: ${cmd}`);
}

await main();
