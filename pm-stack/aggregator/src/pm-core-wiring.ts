/**
 * pm-core scheduler wiring (#8570): binds the scheduler seam
 * (src/pm-core-scheduler.ts, PR #12905) to the aggregator's real jobs —
 * the quota gatherer and the fleet-feed collector, folded into the
 * aggregator process as in-process schedulers. server.ts starts this
 * in live mode only; the systemd timer-unit cutover is a later slice.
 *
 * Both job modules load lazily at tick time, never at server startup:
 * scripts/collect-fleet-feed.ts resolves its output path at module top
 * level (resolveOutPath() reads process.argv and can process.exit(2)
 * on unknown arguments), so importing it into the server's module
 * graph would put the whole aggregator at the mercy of the server's
 * own argv. The quota gatherer loads the same way for symmetry —
 * startup stays free of script module side effects.
 */
import { access, appendFile, mkdir } from "node:fs/promises";
import { dirname, join } from "node:path";
import {
  createPmCoreScheduler,
  type JobResult,
  type PmCoreScheduler,
  type SchedulerAudit,
} from "./pm-core-scheduler.ts";

export interface PmCoreJobs {
  runQuota: () => Promise<void>;
  runFleetFeed: () => Promise<JobResult>;
}

export interface PmCoreJobDeps {
  /** The environment the fleet-feed job pins its output path into. */
  env?: Record<string, string | undefined>;
  /** A present file pauses collector ticks for the laptop chaos drill. */
  feedPauseFile?: string;
  /** Injectable for tests; defaults to the gatherer's own main(). */
  loadQuotaMain?: () => Promise<() => Promise<number>>;
  /** Injectable for tests; defaults to the collector's own main(). */
  loadFleetFeedMain?: () => Promise<() => Promise<void>>;
}

export function createPmCoreJobs(options: { feedFile: string } & PmCoreJobDeps): PmCoreJobs {
  const env = options.env ?? process.env;
  const feedPauseFile = options.feedPauseFile ?? env.PM_CORE_FEED_PAUSE_FILE ?? join(dirname(options.feedFile), "fleet-feed.pause");
  const loadQuotaMain = options.loadQuotaMain ?? (async () => (await import("../scripts/gather-quota.ts")).main);
  const loadFleetFeedMain =
    options.loadFleetFeedMain ?? (async () => (await import("../scripts/collect-fleet-feed.ts")).main);

  return {
    /** The gatherer's main() reports a failed tick as a non-zero exit
     *  code instead of throwing; the scheduler only records an `error`
     *  audit for a thrown failure, so translate the code into a throw. */
    async runQuota() {
      const main = await loadQuotaMain();
      const exitCode = await main();
      if (exitCode !== 0) {
        throw new Error(`quota gatherer reported failure (exit ${exitCode})`);
      }
    },
    async runFleetFeed(): Promise<JobResult> {
      try {
        await access(feedPauseFile);
        return { skipped: `fleet-feed paused by ${feedPauseFile}` };
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
      }
      // The collector resolves its output path at module top level from
      // FLEET_FEED_OUT, falling back to a checkout-relative path — so
      // the pin must be in place before the module is first imported.
      // The board reads cfg.feedFile; the in-process collector must
      // publish exactly there. An operator-provided pin always wins.
      if (!env.FLEET_FEED_OUT) env.FLEET_FEED_OUT = options.feedFile;
      const main = await loadFleetFeedMain();
      await main();
    },
  };
}

type Timer = ReturnType<typeof setInterval>;

export interface SignalTarget {
  once(signal: string, listener: () => void): unknown;
  removeListener(signal: string, listener: () => void): unknown;
}

export interface PmCoreRuntime {
  scheduler: PmCoreScheduler;
  /** Stop the schedulers and detach the signal handlers. Idempotent. */
  shutdown(): void;
}

export function startPmCoreSchedulers(options: {
  feedFile: string;
  /** Write-only audit trail for scheduler attempts. Defaults to
   *  pm-core-scheduler-audit.jsonl beside the feed file — the
   *  aggregator state dir when deployed. */
  auditFile?: string;
  jobDeps?: PmCoreJobDeps;
  writeAudit?: (record: SchedulerAudit) => Promise<void> | void;
  setInterval?: (callback: () => void, delayMs: number) => Timer;
  clearInterval?: (timer: Timer) => void;
  signalTarget?: SignalTarget;
  resignal?: (signal: "SIGTERM" | "SIGINT") => void;
}): PmCoreRuntime {
  const jobs = createPmCoreJobs({ feedFile: options.feedFile, ...options.jobDeps });
  const auditFile = options.auditFile ?? join(dirname(options.feedFile), "pm-core-scheduler-audit.jsonl");
  // The seam guards audit-sink failures itself (writeAuditSafely), so
  // this writer may throw freely: a failed append loses one record,
  // never the tick.
  const writeAudit =
    options.writeAudit ??
    (async (record: SchedulerAudit) => {
      await mkdir(dirname(auditFile), { recursive: true });
      await appendFile(auditFile, `${JSON.stringify(record)}\n`, "utf8");
    });
  // Match the server's other owned loops: the timers never keep the
  // process alive on their own.
  const setTimer =
    options.setInterval ??
    ((callback, delayMs) => {
      const timer = setInterval(callback, delayMs);
      timer.unref?.();
      return timer;
    });

  const scheduler = createPmCoreScheduler({
    runQuota: jobs.runQuota,
    runFleetFeed: jobs.runFleetFeed,
    writeAudit,
    setInterval: setTimer,
    clearInterval: options.clearInterval,
  });
  scheduler.start();
  // Preserve the previous startPmCoreJobs behaviour: refresh promptly after
  // a restart instead of waiting up to one full scheduler cadence.
  void scheduler.runNow("quota");
  void scheduler.runNow("fleet-feed");

  // Shutdown tied to process signals: stop the schedulers, detach the
  // handlers, then re-send the signal to ourselves so the process
  // still terminates with the default disposition it would have had
  // without a listener — the wiring owns no exit policy of its own.
  const signals: SignalTarget = options.signalTarget ?? process;
  const resignal =
    options.resignal ??
    ((signal) => {
      process.kill(process.pid, signal);
    });
  let stopped = false;
  const shutdown = (): void => {
    if (stopped) return;
    stopped = true;
    scheduler.stop();
    signals.removeListener("SIGTERM", onSigterm);
    signals.removeListener("SIGINT", onSigint);
  };
  const onSigterm = (): void => {
    shutdown();
    resignal("SIGTERM");
  };
  const onSigint = (): void => {
    shutdown();
    resignal("SIGINT");
  };
  signals.once("SIGTERM", onSigterm);
  signals.once("SIGINT", onSigint);

  return { scheduler, shutdown };
}
