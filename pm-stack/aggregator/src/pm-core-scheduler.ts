/**
 * In-process PM-core job scheduling (#8570, first consolidation slice).
 *
 * The jobs are injected so the scheduler owns cadence and single-flight
 * semantics without owning provider or feed logic. Audit records are
 * write-only: the scheduler reports each attempt to the caller, but never
 * reads an audit file as a freshness source.
 */

export const QUOTA_CADENCE_MS = 15 * 60_000;
export const FLEET_FEED_CADENCE_MS = 5 * 60_000;

export type SchedulerJobName = "quota" | "fleet-feed";

export interface SchedulerAudit {
  job: SchedulerJobName;
  startedAt: string;
  finishedAt: string;
  status: "ok" | "error";
  error?: string;
}

export interface PmCoreScheduler {
  start(): void;
  stop(): void;
  runNow(job: SchedulerJobName): Promise<void>;
  running(job: SchedulerJobName): boolean;
}

type Timer = ReturnType<typeof setInterval>;
type Job = () => Promise<void> | void;

export function createPmCoreScheduler(options: {
  runQuota: Job;
  runFleetFeed: Job;
  writeAudit: (record: SchedulerAudit) => Promise<void> | void;
  now?: () => Date;
  setInterval?: (callback: () => void, delayMs: number) => Timer;
  clearInterval?: (timer: Timer) => void;
}): PmCoreScheduler {
  const now = options.now ?? (() => new Date());
  const setTimer = options.setInterval ?? ((callback, delayMs) => setInterval(callback, delayMs));
  const clearTimer = options.clearInterval ?? ((timer) => clearInterval(timer));
  const jobs: Record<SchedulerJobName, Job> = {
    quota: options.runQuota,
    "fleet-feed": options.runFleetFeed,
  };
  const cadence: Record<SchedulerJobName, number> = {
    quota: QUOTA_CADENCE_MS,
    "fleet-feed": FLEET_FEED_CADENCE_MS,
  };
  const active = new Set<SchedulerJobName>();
  const timers = new Map<SchedulerJobName, Timer>();

  // A failing audit sink must never escape the scheduler: the interval
  // callback discards runNow's promise, so an escaped rejection would
  // surface as an unhandled rejection in the hosting process, and an
  // audit failure must not mask the job's own outcome either. There is
  // nowhere further to report an audit failure to — the next run retries
  // the sink with its own record.
  async function writeAuditSafely(record: SchedulerAudit): Promise<void> {
    try {
      await options.writeAudit(record);
    } catch {
      // Swallowed by design; see the comment above.
    }
  }

  async function runNow(job: SchedulerJobName): Promise<void> {
    if (active.has(job)) return;
    active.add(job);
    const startedAt = now().toISOString();
    try {
      await jobs[job]();
      await writeAuditSafely({ job, startedAt, finishedAt: now().toISOString(), status: "ok" });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      await writeAuditSafely({
        job,
        startedAt,
        finishedAt: now().toISOString(),
        status: "error",
        error: message,
      });
    } finally {
      active.delete(job);
    }
  }

  return {
    start() {
      if (timers.size > 0) return;
      for (const job of Object.keys(jobs) as SchedulerJobName[]) {
        timers.set(
          job,
          setTimer(() => void runNow(job), cadence[job]),
        );
      }
    },
    stop() {
      for (const timer of timers.values()) clearTimer(timer);
      timers.clear();
    },
    runNow,
    running: (job) => active.has(job),
  };
}
