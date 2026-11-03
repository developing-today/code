/**
 * The remembered daemon choice, on ctx.storage instead of a state file
 * (design doc §4). The host namespaces storage per plugin id, so one key per
 * working directory is the whole scheme; the v1 rememberFile knob and the
 * read/write trio in daemon.ts are not used by the v2 tree.
 *
 * `until-gone` lifetime is preserved the v1 way: the record carries when it
 * was seen, and a caller that health-checks the target drops the record when
 * the check fails (see recallDaemon).
 */
import type { Target } from "../mcpx/daemon.ts";
import { targetKey } from "../mcpx/daemon.ts";

/** The slice of ctx.storage this module needs (structural, host-provided). */
export type StorageLike = {
  get(key: string): Promise<unknown>;
  set(key: string, value: unknown): Promise<void>;
  remove(key: string): Promise<void>;
};

export type RememberedDaemon = {
  target: Target;
  socket?: string;
  seenAt: number;
};

const keyFor = (directory: string): string => `daemon:${directory}`;

export async function rememberDaemon(
  storage: StorageLike,
  directory: string,
  target: Target,
  now: number = Date.now(),
): Promise<void> {
  const record: RememberedDaemon = {
    target,
    socket: target.kind === "socket" ? target.path : undefined,
    seenAt: now,
  };
  await storage.set(keyFor(directory), record);
}

export async function readRememberedDaemon(
  storage: StorageLike,
  directory: string,
): Promise<RememberedDaemon | undefined> {
  const raw = await storage.get(keyFor(directory));
  if (raw === undefined || raw === null || typeof raw !== "object") return undefined;
  const record = raw as RememberedDaemon;
  if (typeof record.seenAt !== "number" || record.target === undefined) return undefined;
  return record;
}

/**
 * Read the remembered daemon and keep it only while `healthy` says its
 * target still answers; a failed check forgets the record (until-gone).
 */
export async function recallDaemon(
  storage: StorageLike,
  directory: string,
  healthy: (target: Target) => Promise<boolean>,
): Promise<RememberedDaemon | undefined> {
  const record = await readRememberedDaemon(storage, directory);
  if (record === undefined) return undefined;
  if (await healthy(record.target)) return record;
  await forgetDaemon(storage, directory);
  return undefined;
}

export async function forgetDaemon(
  storage: StorageLike,
  directory: string,
): Promise<void> {
  await storage.remove(keyFor(directory));
}

export { targetKey };
