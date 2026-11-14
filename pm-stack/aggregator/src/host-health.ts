// Host-health snapshot (#10141): the reader half of silent-failure
// surfacing. pm-stack/host-health-collector.py runs on the laptop as a
// user timer (5 min cadence) and writes one JSON snapshot of the host
// facts the close-authority design (code#10136, P5) says must never be
// silent: the opencode claimant's mode file, and the systemd user unit
// states for pm-git-sync, pm-projector-reconcile, pm-kaneo-sync-guard
// and pm-dashboard-backup (plus their timers).
//
// The aggregator only READS the snapshot — the watchdog derivation in
// watchdog.ts turns breaches into NEEDS ORCHESTRATOR tier items, and
// recovery clears them on the next derivation, exactly like the
// staleness watchdog (#1148). Nothing here is persisted by the
// aggregator, and nothing here invents a state: an absent or corrupt
// snapshot reads as null and the watchdog says so, out loud.
import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";

/** Collector cadence is 5 min; 3 missed cycles (15 min) = stale. */
export const HOST_HEALTH_STALE_MS = 15 * 60_000;
/** pm-dashboard-backup runs daily (~04:30); stale past 1.5 cadences. */
export const HOST_HEALTH_BACKUP_STALE_MS = 36 * 60 * 60_000;
/** pm-projector-reconcile normally finishes in seconds on a 15 min
 *  timer; sitting in `activating` past 10 min means it is stuck. */
export const HOST_HEALTH_RECONCILE_STUCK_MS = 10 * 60_000;

export type ClaimantModeState = "live" | "shadow" | "missing" | "unreadable" | "invalid" | "unknown";

export interface ClaimantModeHealth {
  state: ClaimantModeState;
  detail?: string;
}

export interface HostUnitHealth {
  /** systemctl ActiveState ("active" | "inactive" | "failed" | "activating" | ...). */
  activeState: string;
  subState?: string | null;
  /** systemctl Result of the last run ("success" | "exit-code" | ...). */
  result?: string | null;
  execMainStatus?: number | null;
  stateChangedAt?: string | null;
  lastSuccessAt?: string | null;
  /** Set when the collector itself could not read the unit. */
  state?: string;
  detail?: string;
}

export interface HostHealthSnapshot {
  generatedAt: string;
  claimantMode: ClaimantModeHealth;
  units: Record<string, HostUnitHealth>;
  timers: Record<string, { active: boolean | null; activeState?: string }>;
}

/** Default snapshot path (mirrors src/config.ts hostHealthFile). */
export function defaultHostHealthFile(env: Record<string, string | undefined> = process.env): string {
  const v = env.PM_AGG_HOST_HEALTH_FILE?.trim();
  return v ? v : join(homedir(), ".local", "state", "pm-aggregator", "host-health.json");
}

const CLAIMANT_STATES: readonly string[] = ["live", "shadow", "missing", "unreadable", "invalid", "unknown"];

function asRecord(v: unknown): Record<string, unknown> | null {
  return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
}

function strOrNull(v: unknown): string | null {
  return typeof v === "string" && v ? v : null;
}

/** Validate a parsed snapshot loosely: known-bad shapes read as null
 *  (absent), never as a partial truth the watchdog would trust. */
export function normalizeHostHealthSnapshot(parsed: unknown): HostHealthSnapshot | null {
  const root = asRecord(parsed);
  if (!root) return null;
  const generatedAt = strOrNull(root.generatedAt);
  if (!generatedAt || Number.isNaN(Date.parse(generatedAt))) return null;
  const cm = asRecord(root.claimantMode);
  if (!cm || typeof cm.state !== "string" || !CLAIMANT_STATES.includes(cm.state)) return null;
  const unitsRaw = asRecord(root.units);
  if (!unitsRaw) return null;
  const units: Record<string, HostUnitHealth> = {};
  for (const [id, raw] of Object.entries(unitsRaw)) {
    const u = asRecord(raw);
    if (!u) return null;
    units[id] = {
      activeState: typeof u.activeState === "string" ? u.activeState : "unknown",
      subState: strOrNull(u.subState),
      result: strOrNull(u.result),
      execMainStatus: typeof u.execMainStatus === "number" ? u.execMainStatus : null,
      stateChangedAt: strOrNull(u.stateChangedAt),
      lastSuccessAt: strOrNull(u.lastSuccessAt),
      ...(typeof u.state === "string" ? { state: u.state } : {}),
      ...(typeof u.detail === "string" ? { detail: u.detail } : {}),
    };
  }
  const timers: HostHealthSnapshot["timers"] = {};
  const timersRaw = asRecord(root.timers) ?? {};
  for (const [id, raw] of Object.entries(timersRaw)) {
    const t = asRecord(raw);
    if (!t) continue;
    timers[id] = {
      active: typeof t.active === "boolean" ? t.active : null,
      ...(typeof t.activeState === "string" ? { activeState: t.activeState } : {}),
    };
  }
  return {
    generatedAt,
    claimantMode: {
      state: cm.state as ClaimantModeState,
      ...(typeof cm.detail === "string" ? { detail: cm.detail } : {}),
    },
    units,
    timers,
  };
}

/** Read + normalize the snapshot; null when absent/corrupt (the
 *  watchdog turns that into its own visible item in live mode). */
export async function readHostHealthSnapshot(file: string): Promise<HostHealthSnapshot | null> {
  try {
    return normalizeHostHealthSnapshot(JSON.parse(await readFile(file, "utf8")));
  } catch {
    return null;
  }
}
