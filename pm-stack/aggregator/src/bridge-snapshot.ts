// Opt-in, credential-free projection for the private Fleet laptop bridge.
// This does not poll, execute commands, enroll a device or change services.
import { constants } from "node:fs";
import { lstat, mkdir, open, rename, unlink } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { homedir } from "node:os";
import { join } from "node:path";
import type { QueueSnapshot } from "./queue.ts";

export const BRIDGE_STATUSES = ["queued", "claimed", "parked", "handed_off", "blocked", "done", "cancelled", "failed"] as const;
type Status = (typeof BRIDGE_STATUSES)[number];
export interface BridgeQueueSnapshot {
  version: 1;
  fetchedAt: number;
  available: boolean;
  stale: boolean;
  // The collector discards broker pagination totals. Never claim completeness,
  // even for an empty page; health counts are not a consistent list snapshot.
  complete: false;
  total: null;
  tasks: { id: number; status: Status; priority: number | null }[];
  counts: Record<Status, number>;
}
const counts = (): Record<Status, number> => Object.fromEntries(BRIDGE_STATUSES.map((s) => [s, 0])) as Record<Status, number>;
const nonnegative = (n: unknown): n is number => Number.isSafeInteger(n) && (n as number) >= 0;

/** Allowlist output fields, never copy free-form content or error messages. */
export function bridgeQueueSnapshot(source: QueueSnapshot, now = Date.now()): BridgeQueueSnapshot {
  const unavailable = (): BridgeQueueSnapshot => ({
    version: 1, fetchedAt: now, available: false, stale: true,
    complete: false, total: null, tasks: [], counts: counts(),
  });
  if (source.bridgeVerified !== true || source.available !== true || source.stale !== false
      || !Array.isArray(source.errors) || source.errors.length !== 0 || !nonnegative(source.fetchedAt)
      || source.fetchedAt > now + 5_000 || now - source.fetchedAt > 300_000
      || !Array.isArray(source.tasks) || !Array.isArray(source.queues)) return unavailable();
  const result: BridgeQueueSnapshot = {
    version: 1, fetchedAt: source.fetchedAt, available: true, stale: false,
    complete: false, total: null, tasks: [], counts: counts(),
  };
  const ids = new Set<number>();
  for (const task of source.tasks) {
    if (!nonnegative(task.id) || task.id === 0 || ids.has(task.id)
        || !BRIDGE_STATUSES.includes(task.status as Status)
        || !(task.priority === null || (typeof task.priority === "number" && Number.isFinite(task.priority)))) return unavailable();
    ids.add(task.id);
    if (result.tasks.length < 1000) result.tasks.push({ id: task.id, status: task.status as Status, priority: task.priority });
  }
  for (const row of source.queues) {
    for (const state of BRIDGE_STATUSES) {
      const n = row[state === "handed_off" ? "handedOff" : state];
      if (!nonnegative(n) || !nonnegative(result.counts[state] + n)) return unavailable();
      result.counts[state] += n;
    }
  }
  return result;
}

/** Only the literal opt-in enables writes. home is injected by offline tests;
 * production uses homedir(), never a caller argument or path environment flag.
 * Existing directory ownership and symlinks are checked; only a new private
 * temp file is opened (exclusive + NOFOLLOW), then atomically replaces target.
 * This protects against accidental symlink traversal, not a hostile same-UID
 * process concurrently renaming owned ancestor directories.
 */
export async function publishBridgeQueueSnapshot(
  source: QueueSnapshot,
  enabled = process.env.PM_AGG_BRIDGE_SNAPSHOT,
  home = homedir(),
  now = Date.now(),
): Promise<void> {
  if (enabled !== "1") return;
  let directory = home;
  const check = async () => {
    const st = await lstat(directory);
    if (!st.isDirectory() || st.isSymbolicLink() || (st.mode & 0o022) !== 0
        || (typeof process.getuid === "function" && st.uid !== process.getuid())) throw new Error("bridge snapshot directory refused");
  };
  await check();
  for (const part of [".local", "state", "pm-aggregator"]) {
    directory = join(directory, part);
    await mkdir(directory, { mode: 0o700 }).catch((err: NodeJS.ErrnoException) => { if (err.code !== "EEXIST") throw err; });
    await check();
  }
  const target = join(directory, "bridge-queue.json");
  const existing = await lstat(target).catch((err: NodeJS.ErrnoException) => { if (err.code === "ENOENT") return null; throw err; });
  if (existing && (!existing.isFile() || existing.isSymbolicLink())) throw new Error("bridge snapshot target refused");
  const temp = join(directory, `.bridge-queue-${randomUUID()}.tmp`);
  const file = await open(temp, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW, 0o600);
  try {
    await file.writeFile(JSON.stringify(bridgeQueueSnapshot(source, now)) + "\n");
    await file.sync();
    await file.close();
    await rename(temp, target);
  } finally {
    await file.close().catch(() => {});
    await unlink(temp).catch(() => {});
  }
}
