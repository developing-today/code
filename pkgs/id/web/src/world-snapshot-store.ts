/**
 * #10927 (shells v1 slice 1): the browser shell's snapshot store —
 * the persistence half of the PWA offline read model.
 *
 * The shell keeps a local snapshot (last applied journal sequence +
 * rendered state) sufficient to READ the world fully offline. The
 * store is keyed per host and per world name ("" is the host's
 * default world). Entries are validated on load with the same shape
 * discipline as the world frame validators in world.ts — safe
 * non-negative integer sequences and participant ids, string fields,
 * a screen surface the renderer would accept — because localStorage
 * is shared, user-writable state: a corrupt or foreign entry is
 * ignored, never rendered and never adopted as the resume cursor.
 *
 * Storage failures (quota, privacy mode) are swallowed: the offline
 * store is best-effort and must never break the live session.
 */

import type { ScreenSurface } from "./world-surface";

export type StoredWorldView = { kind: "surface"; surface: ScreenSurface } | { kind: "text"; text: string };

export type StoredWorldSnapshot = {
  world_id: string;
  sequence: number;
  participants: Array<{ id: number; display_name: string }>;
  view: StoredWorldView | null;
  view_revision: number | null;
  records: Record<string, unknown>;
};

export function worldSnapshotStoreKey(host: string, world: string | null): string {
  return `id-world-snapshot:${host}:${world ?? ""}`;
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const isSafeNonNegativeInteger = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0;

const isStoredSurface = (value: unknown): value is ScreenSurface =>
  isRecord(value) &&
  value.kind === "screen" &&
  value.version === 1 &&
  Number.isInteger(value.width) &&
  Number.isInteger(value.height) &&
  (value.width as number) >= 1 &&
  (value.height as number) >= 1 &&
  Array.isArray(value.cells) &&
  value.cells.length === (value.width as number) * (value.height as number);

const isStoredView = (value: unknown): value is StoredWorldView | null => {
  if (value === null) return true;
  if (!isRecord(value)) return false;
  if (value.kind === "text") return typeof value.text === "string";
  if (value.kind === "surface") return isStoredSurface(value.surface);
  return false;
};

export function parseStoredWorldSnapshot(raw: string | null): StoredWorldSnapshot | null {
  if (raw === null) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw) as unknown;
  } catch {
    return null;
  }
  if (
    !isRecord(parsed) ||
    typeof parsed.world_id !== "string" ||
    !isSafeNonNegativeInteger(parsed.sequence) ||
    !Array.isArray(parsed.participants) ||
    !parsed.participants.every(
      (participant) =>
        isRecord(participant) &&
        isSafeNonNegativeInteger(participant.id) &&
        typeof participant.display_name === "string",
    ) ||
    !isStoredView(parsed.view) ||
    !(parsed.view_revision === null || (typeof parsed.view_revision === "number" && Number.isFinite(parsed.view_revision))) ||
    !isRecord(parsed.records)
  ) {
    return null;
  }
  return parsed as StoredWorldSnapshot;
}

export function loadWorldSnapshot(storage: Storage, host: string, world: string | null): StoredWorldSnapshot | null {
  try {
    return parseStoredWorldSnapshot(storage.getItem(worldSnapshotStoreKey(host, world)));
  } catch {
    return null;
  }
}

export function saveWorldSnapshot(
  storage: Storage,
  host: string,
  world: string | null,
  snapshot: StoredWorldSnapshot,
): void {
  try {
    storage.setItem(worldSnapshotStoreKey(host, world), JSON.stringify(snapshot));
  } catch {
    // Best-effort: a full or unavailable store never breaks the session.
  }
}
