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
  if (check.alert !== "needs-orchestrator") return;
  // Stable for one outage, but distinct after a recovery and later failure.
  const key = `watchdog-heartbeat-${check.outageSinceMs}-needs-orchestrator`;
  const title = "NEEDS ORCHESTRATOR: PM-stack watchdog missed two heartbeat cycles";
  const body = check.lastHeartbeatAtMs === null
    ? `No valid watchdog heartbeat has ever been recorded; watchdog journal activity began at ${new Date(check.outageSinceMs!).toISOString()}. ${check.missedCycles} five-minute cycles have elapsed.`
    : `The last valid watchdog heartbeat was ${new Date(check.lastHeartbeatAtMs).toISOString()}. ${check.missedCycles} five-minute cycles have been missed.`;
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
      "warning",
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
  if (result.alert === "unknown") {
    console.log(`unknown: no valid watchdog heartbeat or journal activity in ${journal}`);
    return;
  }
  if (result.alert === "page-drew") {
    // This script runs under the independent page-health-push job, whose
    // completed report is delivered to the main chat. Keep the critical page
    // on that out-of-process channel: post-alert.ts calls back into this
    // aggregator and therefore cannot deliver when the aggregator is down.
    const reference = result.lastHeartbeatAtMs === null
      ? `No watchdog heartbeat has ever been recorded; journal activity began at ${new Date(result.outageSinceMs!).toISOString()}.`
      : `The last valid watchdog heartbeat was ${new Date(result.lastHeartbeatAtMs).toISOString()}.`;
    console.log(`PAGE_DREW: PM-stack watchdog heartbeat missing. ${reference} ${result.missedCycles} five-minute cycles have been missed.`);
    return;
  }
  await postAlert(result);
  if (result.alert === "needs-orchestrator") console.log(result.alert);
}

await main();
