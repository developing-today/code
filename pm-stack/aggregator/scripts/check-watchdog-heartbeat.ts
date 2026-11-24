#!/usr/bin/env bun
// Independent five-minute watchdog heartbeat checker. The external
// page-health-push runner invokes this script; it reads only the SLO
// journal, then publishes any alert on stdout for that runner's
// delivered report (the channel that still works when the aggregator
// is down). The two-cycle alert is additionally recorded as an
// incident in the aggregator's alerts ledger through post-alert.ts.
//
// Usage: bun run scripts/check-watchdog-heartbeat.ts [--journal <path>]
// Env: PM_AGG_SLO_JOURNAL_FILE (default: ~/.local/state/pm-aggregator/slo-journal.jsonl)
// Every option is parsed strictly before the journal is read or a subprocess
// is spawned. The caller must schedule this independently of the aggregator.
import { readFile, writeFile } from "node:fs/promises";
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

/** The checker owns a small state file beside the journal holding the
 *  monitoring activation timestamp: when heartbeat monitoring was first
 *  observed active against this journal. The journal itself predates
 *  heartbeat monitoring, so its oldest sample cannot provide the
 *  startup grace; the persisted activation can. */
function checkerStatePath(journal: string): string {
  return `${journal}.checker-state.json`;
}

async function readActivationAtMs(journal: string): Promise<number | null> {
  try {
    const parsed: unknown = JSON.parse(await readFile(checkerStatePath(journal), "utf8"));
    if (
      typeof parsed === "object" &&
      parsed !== null &&
      "activationAtMs" in parsed &&
      typeof parsed.activationAtMs === "number" &&
      Number.isFinite(parsed.activationAtMs)
    ) {
      return parsed.activationAtMs;
    }
  } catch {
    // Missing or corrupt state means monitoring activation was never
    // persisted for this journal; the caller treats this run as the
    // first observation and re-persists.
  }
  return null;
}

async function persistActivationAtMs(journal: string, activationAtMs: number): Promise<void> {
  try {
    await writeFile(checkerStatePath(journal), `${JSON.stringify({ activationAtMs })}\n`, "utf8");
  } catch (error) {
    // Without persistence every run looks like the first observation
    // and a never-beating watchdog would never escalate. Surface it in
    // the delivered report instead of failing the check itself.
    console.error(
      `check-watchdog-heartbeat: WARNING: could not persist monitoring activation to ${checkerStatePath(journal)}: ${error instanceof Error ? error.message : String(error)}`,
    );
  }
}

async function postAlert(check: WatchdogHeartbeatCheck): Promise<void> {
  if (check.alert !== "needs-orchestrator") return;
  // Stable for one outage, but distinct after a recovery and later failure.
  const key = `watchdog-heartbeat-${check.outageSinceMs}-needs-orchestrator`;
  const title = "NEEDS ORCHESTRATOR: PM-stack watchdog missed two heartbeat cycles";
  const body =
    check.lastHeartbeatAtMs === null
      ? `No valid watchdog heartbeat has ever been recorded; the outage reference (monitoring activation or first journal activity, whichever is later) is ${new Date(check.outageSinceMs!).toISOString()}. ${check.missedCycles} five-minute cycles have elapsed.`
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
  const nowMs = Date.now();
  // Startup grace is based on monitoring activation, not on the
  // journal's oldest sample: with no persisted activation, this run is
  // the first observation, so absence is measured from now and the
  // activation is persisted once this run confirms the journal has
  // activity but no heartbeat has ever been recorded.
  const persistedActivationAtMs = await readActivationAtMs(journal);
  const result = await checkWatchdogHeartbeatJournal(journal, nowMs, undefined, {
    activationAtMs: persistedActivationAtMs ?? nowMs,
  });
  if (persistedActivationAtMs === null && result.lastHeartbeatAtMs === null && result.alert !== "unknown") {
    await persistActivationAtMs(journal, nowMs);
  }
  if (result.alert === "unknown") {
    console.log(`unknown: no valid watchdog heartbeat or journal activity in ${journal}`);
    return;
  }
  const reference =
    result.lastHeartbeatAtMs === null
      ? `No watchdog heartbeat has ever been recorded; the outage reference is ${new Date(result.outageSinceMs!).toISOString()}.`
      : `The last valid watchdog heartbeat was ${new Date(result.lastHeartbeatAtMs).toISOString()}.`;
  if (result.alert === "page-drew") {
    // This script runs under the independent page-health-push job, whose
    // completed report is delivered to the main chat. Keep the critical page
    // on that out-of-process channel: post-alert.ts calls back into this
    // aggregator and therefore cannot deliver when the aggregator is down.
    console.log(
      `PAGE_DREW: PM-stack watchdog heartbeat missing. ${reference} ${result.missedCycles} five-minute cycles have been missed.`,
    );
    return;
  }
  if (result.alert === "needs-orchestrator") {
    // Publish the escalation on the same independent channel as the
    // page, before the ledger write: the page-health-push report reaches
    // the orchestrator's main chat whether or not the aggregator is up,
    // and a failed post-alert must not lose this line. The post-alert
    // incident is the ledger record; a warning incident alone is never
    // delivered (the alert engine delivers criticals only) and the
    // attention feed's orchestrator tier is derived inside the
    // aggregator, so the incident title by itself routes nothing.
    console.log(
      `NEEDS_ORCHESTRATOR: PM-stack watchdog heartbeat missing. ${reference} ${result.missedCycles} five-minute cycles have been missed.`,
    );
  }
  await postAlert(result);
}

await main();
