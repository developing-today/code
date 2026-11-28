/** Own the quota and fleet-feed jobs from the aggregator process (#8570).
 *
 * The legacy oneshot scripts remain executable for rollback/manual runs, but
 * live pm-core imports their exported entry points and owns their cadence.
 * Their normal output files remain the jobs' responsibility; this audit log
 * records scheduler outcomes only and is never read for freshness.
 */
import { appendFile, mkdir } from "node:fs/promises";
import { dirname, join } from "node:path";
import { homedir } from "node:os";
import { createPmCoreScheduler, type SchedulerAudit } from "./pm-core-scheduler.ts";

const DEFAULT_AUDIT_PATH = join(homedir(), ".local/state/pm-aggregator/pm-core-jobs.jsonl");

export async function startPmCoreJobs(
  options: { auditPath?: string; warn?: (message: string) => void } = {},
): Promise<ReturnType<typeof createPmCoreScheduler>> {
  // Dynamic imports keep fixture/dev tools from loading host-only job
  // dependencies unless this live process actually starts the schedulers.
  const [{ main: runQuota }, { main: runFleetFeed }] = await Promise.all([
    import("../scripts/gather-quota.ts"),
    import("../scripts/collect-fleet-feed.ts"),
  ]);
  const auditPath = options.auditPath ?? process.env.PM_CORE_JOBS_AUDIT_PATH ?? DEFAULT_AUDIT_PATH;
  const scheduler = createPmCoreScheduler({
    runQuota: async () => {
      const code = await runQuota();
      if (code !== 0) throw new Error(`quota gatherer exited with status ${code}`);
    },
    runFleetFeed,
    writeAudit: async (record: SchedulerAudit) => {
      await mkdir(dirname(auditPath), { recursive: true });
      await appendFile(auditPath, `${JSON.stringify(record)}\n`, "utf8");
    },
  });
  scheduler.start();
  // Preserve the old oneshot behavior: refresh promptly after service start,
  // then let the scheduler own the established 15m/5m intervals.
  void scheduler.runNow("quota");
  void scheduler.runNow("fleet-feed");
  options.warn?.("pm-core owns quota (15m) and fleet-feed (5m) schedules");
  return scheduler;
}
