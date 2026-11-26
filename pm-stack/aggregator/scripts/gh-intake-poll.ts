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
import {
  INTAKE_REPOS,
  advanceWatermark,
  isValidIsoTimestamp,
  loadState,
  saveState,
  selectNewIssues,
  type IntakeItem,
} from "../src/intake.ts";

function usageError(message: string): never {
  console.error(`gh-intake-poll: ${message}`);
  console.error("usage: gh-intake-poll.ts poll [--state <file>] [--since <iso>] [--repos a/b,c/d]");
  console.error("       gh-intake-poll.ts advance --to <iso> [--state <file>]");
  process.exit(2);
}

function help(): void {
  console.log("gh-intake-poll: phase-1 new-issue intake poll for #1480");
  console.log("usage: gh-intake-poll.ts poll [--state <file>] [--since <iso>] [--repos a/b,c/d]");
  console.log("       gh-intake-poll.ts advance --to <iso> [--state <file>]");
  console.log("poll is read-only; advance moves the watermark forward only");
}

/** Parse the entire subcommand line before state reads, network calls, or writes. */
function parseOptions(args: string[], allowed: readonly string[]): Map<string, string> {
  const options = new Map<string, string>();
  for (let i = 0; i < args.length; ) {
    const name = args[i]!;
    if (!allowed.includes(name)) usageError(`unexpected argument: ${name}`);
    if (options.has(name)) usageError(`${name} may only be specified once`);
    const value = args[i + 1];
    if (value == null || value.length === 0 || value.startsWith("--")) {
      usageError(`${name} needs a non-empty value`);
    }
    options.set(name, value);
    i += 2;
  }
  return options;
}

function ghApi(path: string): unknown {
  const proc = Bun.spawnSync(["gh", "api", path], { stdout: "pipe", stderr: "pipe" });
  if (proc.exitCode !== 0) {
    throw new Error(`gh api ${path} failed (${proc.exitCode}): ${new TextDecoder().decode(proc.stderr).slice(0, 400)}`);
  }
  return JSON.parse(new TextDecoder().decode(proc.stdout));
}

function resolveStateFile(options: Map<string, string>): string {
  return options.get("--state") ?? process.env.PM_INTAKE_STATE_FILE ?? "intake-state.json";
}

async function readWatermark(stateFile: string, sinceOverride: string | undefined): Promise<string> {
  if (sinceOverride !== undefined) {
    if (!isValidIsoTimestamp(sinceOverride)) usageError(`--since is not a valid timestamp: ${sinceOverride}`);
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
  const maxPages = 10;
  for (let page = 1; page <= maxPages; page++) {
    const batch = ghApi(
      `repos/${repo}/issues?state=all&sort=created&direction=desc&per_page=100&page=${page}`,
    ) as Record<string, unknown>[];
    if (!Array.isArray(batch) || batch.length === 0) break;
    const oldest = batch[batch.length - 1]?.created_at;
    const reachedWatermark =
      typeof oldest === "string" && Number.isFinite(Date.parse(oldest)) && Date.parse(oldest) <= mark;
    if (page === maxPages && batch.length === 100 && !reachedWatermark) {
      throw new Error(
        `poll exceeded ${maxPages * 100} issue results for ${repo} before reaching the watermark; refusing an incomplete window`,
      );
    }
    raw.push(...batch);
    if (reachedWatermark) break;
    if (batch.length < 100) break;
  }
  return selectNewIssues(repo, raw, watermark);
}

async function cmdPoll(options: Map<string, string>): Promise<void> {
  const stateFile = resolveStateFile(options);
  const since = options.get("--since");
  const reposCsv = options.get("--repos");
  const repos = reposCsv === undefined ? [...INTAKE_REPOS] : reposCsv.split(",").map((s) => s.trim());
  if (repos.some((repo) => repo.length === 0)) usageError("--repos contains an empty repo entry");
  if (new Set(repos).size !== repos.length) usageError("--repos contains a duplicate repo");
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

async function cmdAdvance(options: Map<string, string>): Promise<void> {
  const to = options.get("--to") ?? usageError("advance needs --to <iso>");
  if (!isValidIsoTimestamp(to)) usageError(`--to is not a valid timestamp: ${to}`);
  const stateFile = resolveStateFile(options);
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
  if ((cmd === "--help" || cmd === "-h") && rest.length === 0) {
    help();
    return;
  }
  if ((cmd === "poll" || cmd === "advance") && rest.length === 1 && (rest[0] === "--help" || rest[0] === "-h")) {
    help();
    return;
  }
  if (cmd === "poll") return cmdPoll(parseOptions(rest, ["--state", "--since", "--repos"]));
  if (cmd === "advance") return cmdAdvance(parseOptions(rest, ["--to", "--state"]));
  usageError(cmd == null ? "missing subcommand (poll | advance)" : `unknown subcommand: ${cmd}`);
}

await main();
