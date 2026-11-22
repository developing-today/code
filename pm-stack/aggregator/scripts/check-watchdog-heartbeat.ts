#!/usr/bin/env bun
// Independent five-minute watchdog heartbeat checker. The external
// page-health-push runner invokes this script; it reads only the SLO
// journal, then sends any alert through the existing post-alert CLI.
//
// Usage: bun run scripts/check-watchdog-heartbeat.ts [--journal <path>]
// Env: PM_AGG_SLO_JOURNAL_FILE (default: ~/.local/state/pm-aggregator/slo-journal.jsonl)
// Every option is parsed strictly before the journal is read or a subprocess
// is spawned. The caller must schedule this independently of the aggregator.
import { homedir } from "node:os";
import { join } from "node:path";
import { checkWatchdogHeartbeatJournal, type WatchdogHeartbeatCheck } from "../src/watchdog-heartbeat-checker.ts";

const USAGE = "usage: check-watchdog-heartbeat.ts [--journal <path>]";
const FLAGS = ["--journal"] as const;

function usageError(message: string): never {
  console.error(`check-watchdog-heartbeat: ${message}`);
  console.error(USAGE);
  process.exit(2);
}

function parseArgs(args: readonly string[]): { journal: string | undefined } {
  let journal: string | undefined;
  const seen = new Set<string>();
  for (let i = 0; i < args.length; i++) {
    const flag = args[i] as string;
    if (!FLAGS.includes(flag as (typeof FLAGS)[number])) usageError(`unknown argument ${JSON.stringify(flag)}`);
    if (seen.has(flag)) usageError(`duplicate ${flag}`);
    seen.add(flag);
    const value = args[++i];
    if (value === undefined || value === "" || value.startsWith("--")) {
      usageError(`${flag} requires a value that is not a flag`);
    }
    journal = value;
  }
  return { journal };
}

async function postAlert(check: WatchdogHeartbeatCheck): Promise<void> {
  if (check.alert !== "needs-orchestrator" && check.alert !== "page-drew") return;
  const isPage = check.alert === "page-drew";
  const key = isPage ? "watchdog-heartbeat-page-drew" : "watchdog-heartbeat-needs-orchestrator";
  const title = isPage
    ? "PM-stack watchdog heartbeat missing — page Drew"
    : "NEEDS ORCHESTRATOR: PM-stack watchdog missed two heartbeat cycles";
  const body = `The last valid watchdog heartbeat was ${new Date(check.lastHeartbeatAtMs!).toISOString()}. ` +
    `${check.missedCycles} five-minute cycles have been missed.`;
  const child = Bun.spawn(
    [
      "bun",
      "run",
      join(import.meta.dir, "post-alert.ts"),
      "--source",
      "pm-stack-watchdog",
      "--key",
      key,
      "--severity",
      isPage ? "critical" : "warning",
      "--title",
      title,
      "--body",
      body,
    ],
    { stdout: "inherit", stderr: "inherit" },
  );
  const code = await child.exited;
  if (code !== 0) throw new Error(`post-alert exited with status ${code}`);
}

async function main(): Promise<void> {
  const args = parseArgs(process.argv.slice(2));
  const defaultJournal = join(homedir(), ".local", "state", "pm-aggregator", "slo-journal.jsonl");
  const journal = args.journal ?? process.env.PM_AGG_SLO_JOURNAL_FILE ?? defaultJournal;
  const result = await checkWatchdogHeartbeatJournal(journal, Date.now());
  if (result.alert === "unknown") throw new Error(`no valid watchdog heartbeat in journal: ${journal}`);
  await postAlert(result);
  if (result.alert !== "none" && result.alert !== "unknown") console.log(result.alert);
}

await main();
