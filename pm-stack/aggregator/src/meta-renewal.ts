/**
 * Meta (dev.meta.ai) session keeper — Drew's provider browser-auth
 * auto-renewal rule (2026-11-05) applied to the one provider whose
 * auth lives ENTIRELY in a browser profile.
 *
 * Why Meta needs a different keeper shape than OpenDesign: both Meta
 * quota read paths — the portal read inside the opencode meta quota
 * provider, and the meta-quota-export file exporter — ride the
 * laptop Helium profile's dev.meta.ai session. There is no stored
 * credential for a keeper to slide or re-capture, and the portal
 * exposes no session expiry to watch. What CAN be watched is the
 * probe the exporter already makes: meta-quota-export evaluates its
 * fetch inside the browser's own page context every 15 minutes and
 * writes the verdict to ~/atlas/meta-quota-export/latest.json.
 *
 * How the last death went (2026-11-01 → 2026-11-05): the exporter's
 * systemd units were renamed to .backup on Nov 1, so nothing even
 * attempted a read; the session expired; the gatherer retained
 * last-known for ~40h with no alert anywhere. This keeper exists so
 * that sequence cannot repeat silently:
 *
 *   1. Every gather tick: read the exporter's latest document.
 *      Fresh + status "ok" = the browser session is valid.
 *   2. Session-expired, or the exporter silent past the stale
 *      horizon: attempt recovery — first verify the exporter units
 *      are armed, restoring them from their .backup names if the
 *      Nov-1 rename is ever found again, then run the exporter once
 *      and re-read its document. A fresh "ok" = recovered.
 *   3. Still expired: the session is truly logged out and re-login
 *      is a human step (Meta emails a login code). The state file
 *      carries an alert from the FIRST bad tick — while last-good
 *      data is still on the board — instead of silence at zero.
 *
 * The state file (meta-renewal.json beside quota-current.json) is
 * rewritten every run. No credential value is read, logged, or
 * written anywhere in this module: the probe document carries no
 * secrets, and the session itself never leaves the browser.
 */

import { execFile } from "node:child_process";
import { access, mkdir, readFile, rename } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

/** Two exporter ticks (2 × 15 min) without a fresh document = stale. */
export const STALE_MS = 30 * 60 * 1000;

export interface MetaExporterDoc {
  status: string | null;
  measuredAt: string | null;
  attemptedAt: string | null;
}

export type EnsureArmedResult =
  | "armed"
  | "restored"
  | "missing"
  /**
   * Partially armed: at least one unit or .backup copy was found, but
   * the pair is not fully armed. The suffix names each unit's actual
   * state, e.g. "partial:timer=restored,service=missing" (#6366).
   */
  | `partial:${string}`;

export interface MetaRenewalState {
  provider: "meta";
  checkedAt: string;
  status: "valid" | "recovered" | "attention";
  exporterStatus: string | null;
  lastGoodAt: string | null;
  recovery: { attempted: boolean; result: string | null };
  alert: string | null;
  alertSince: string | null;
}

export interface MetaRenewalDeps {
  statePath: string;
  latestPath?: string;
  now?: () => Date;
  staleMs?: number;
  /** Injectable for tests; defaults read the exporter's document. */
  readLatest?: () => Promise<MetaExporterDoc | null>;
  ensureExporterArmed?: () => Promise<EnsureArmedResult>;
  runExporter?: () => Promise<void>;
  log?: (line: string) => void;
}

export const defaultLatestPath = (): string => join(homedir(), "atlas", "meta-quota-export", "latest.json");

const defaultReadLatest = async (latestPath: string): Promise<MetaExporterDoc | null> => {
  try {
    const parsed = JSON.parse(await readFile(latestPath, "utf8")) as {
      status?: unknown;
      measuredAt?: unknown;
      attemptedAt?: unknown;
    };
    return {
      status: typeof parsed.status === "string" ? parsed.status : null,
      measuredAt: typeof parsed.measuredAt === "string" ? parsed.measuredAt : null,
      attemptedAt: typeof parsed.attemptedAt === "string" ? parsed.attemptedAt : null,
    };
  } catch {
    return null;
  }
};

const reloadAndEnableExporter = async (): Promise<void> => {
  await execFileAsync("systemctl", ["--user", "daemon-reload"]).catch(() => undefined);
  await execFileAsync("systemctl", ["--user", "enable", "--now", "meta-quota-export.timer"]).catch(() => undefined);
};

const EXPORTER_UNITS = [
  { name: "timer", file: "meta-quota-export.timer" },
  { name: "service", file: "meta-quota-export.service" },
] as const;

/**
 * The default (filesystem) ensure-armed implementation, against an
 * explicit unit directory so tests can exercise it on a fake one.
 * Each unit is handled independently (#6366): a present unit stays,
 * a missing unit with a .backup copy is restored from its own copy,
 * and a unit with neither is reported missing — one unit's state
 * never blocks another unit's restore.
 */
export const ensureExporterArmedForUnitDir = async (
  unitDir: string,
  afterRestore: () => Promise<void> = reloadAndEnableExporter,
): Promise<EnsureArmedResult> => {
  const exists = async (p: string): Promise<boolean> =>
    access(p).then(
      () => true,
      () => false,
    );
  const states = new Map<string, string>();
  let restoredAny = false;
  for (const unit of EXPORTER_UNITS) {
    const path = join(unitDir, unit.file);
    if (await exists(path)) {
      states.set(unit.name, "present");
      continue;
    }
    const backup = `${path}.backup`;
    if (!(await exists(backup))) {
      states.set(unit.name, "missing");
      continue;
    }
    try {
      await rename(backup, path);
      states.set(unit.name, "restored");
      restoredAny = true;
    } catch {
      states.set(unit.name, "backup-unrestorable");
    }
  }
  const allArmed = EXPORTER_UNITS.every((u) => {
    const s = states.get(u.name);
    return s === "present" || s === "restored";
  });
  if (allArmed && !restoredAny) return "armed";
  if (allArmed) {
    await afterRestore().catch(() => undefined);
    return "restored";
  }
  const anyFound = EXPORTER_UNITS.some((u) => states.get(u.name) !== "missing");
  if (!anyFound) return "missing";
  const summary = EXPORTER_UNITS.map((u) => `${u.name}=${states.get(u.name)}`).join(",");
  return `partial:${summary}`;
};

const defaultEnsureExporterArmed = (): Promise<EnsureArmedResult> =>
  ensureExporterArmedForUnitDir(join(homedir(), ".config", "systemd", "user"));

const defaultRunExporter = async (): Promise<void> => {
  await execFileAsync("systemctl", ["--user", "start", "meta-quota-export.service"], {
    timeout: 130_000,
  });
};

const readPreviousState = async (statePath: string): Promise<MetaRenewalState | null> => {
  try {
    return JSON.parse(await readFile(statePath, "utf8")) as MetaRenewalState;
  } catch {
    return null;
  }
};

const writeState = async (statePath: string, state: MetaRenewalState): Promise<void> => {
  await mkdir(dirname(statePath), { recursive: true });
  const tmp = `${statePath}.tmp-${process.pid}`;
  const { writeFile } = await import("node:fs/promises");
  await writeFile(tmp, JSON.stringify(state, null, 2) + "\n", "utf8");
  await rename(tmp, statePath);
};

const isFresh = (doc: MetaExporterDoc | null, now: Date, staleMs: number): boolean => {
  if (!doc?.measuredAt) return false;
  const t = Date.parse(doc.measuredAt);
  return Number.isFinite(t) && now.getTime() - t < staleMs;
};

/**
 * One keeper cycle. Fail-soft by contract: it always resolves with a
 * state object (which it also persists); it never throws, so the
 * gatherer's tick can call it unconditionally.
 */
export async function renewMetaSession(deps: MetaRenewalDeps): Promise<MetaRenewalState> {
  const now = (deps.now ?? (() => new Date()))();
  const staleMs = deps.staleMs ?? STALE_MS;
  const latestPath = deps.latestPath ?? defaultLatestPath();
  const readLatest = deps.readLatest ?? (() => defaultReadLatest(latestPath));
  const ensureExporterArmed = deps.ensureExporterArmed ?? defaultEnsureExporterArmed;
  const runExporter = deps.runExporter ?? defaultRunExporter;
  const log = deps.log ?? (() => undefined);
  const previous = await readPreviousState(deps.statePath);

  const finish = async (
    partial: Omit<MetaRenewalState, "provider" | "checkedAt" | "alertSince">,
  ): Promise<MetaRenewalState> => {
    const state: MetaRenewalState = {
      provider: "meta",
      checkedAt: now.toISOString(),
      alertSince: partial.alert ? (previous?.alert ? previous.alertSince : now.toISOString()) : null,
      ...partial,
    };
    await writeState(deps.statePath, state).catch(() => undefined);
    log(
      `meta-renewal: status=${state.status} exporter=${state.exporterStatus ?? "none"} lastGoodAt=${state.lastGoodAt ?? "never"} recovery=${state.recovery.result ?? "not-attempted"}${state.alert ? ` alert=${state.alert}` : ""}`,
    );
    return state;
  };

  let doc = await readLatest().catch(() => null);
  const lastGoodAt = doc?.status === "ok" && doc.measuredAt ? doc.measuredAt : (previous?.lastGoodAt ?? null);

  if (doc?.status === "ok" && isFresh(doc, now, staleMs)) {
    return finish({
      status: "valid",
      exporterStatus: doc.status,
      lastGoodAt,
      recovery: { attempted: false, result: "not-attempted" },
      alert: null,
    });
  }

  // Recovery: re-arm the exporter if its units were renamed away,
  // then take a fresh probe.
  let recoveryResult: string;
  try {
    const armed = await ensureExporterArmed();
    if (armed === "missing") {
      recoveryResult = "exporter-missing";
    } else if (armed.startsWith("partial:")) {
      recoveryResult = `exporter-partial:${armed.slice("partial:".length)}`;
    } else {
      await runExporter();
      doc = await readLatest().catch(() => null);
      recoveryResult =
        doc?.status === "ok" && isFresh(doc, now, staleMs)
          ? armed === "restored"
            ? "recovered-units-restored"
            : "recovered"
          : "still-expired";
    }
  } catch (error) {
    recoveryResult = `error:${error instanceof Error ? error.message : String(error)}`;
  }

  if (recoveryResult === "recovered" || recoveryResult === "recovered-units-restored") {
    return finish({
      status: "recovered",
      exporterStatus: doc?.status ?? null,
      lastGoodAt: doc?.measuredAt ?? lastGoodAt,
      recovery: { attempted: true, result: recoveryResult },
      alert: null,
    });
  }

  const darkSince = lastGoodAt ?? previous?.lastGoodAt ?? null;
  const alert = recoveryResult.startsWith("exporter-partial:")
    ? `Meta quota exporter units are only partially armed in ~/.config/systemd/user (per-unit state: ${recoveryResult.slice("exporter-partial:".length)}); Meta measurement has no probe until the missing units are restored. Restore the missing units to re-arm the Meta session keeper.`
    : recoveryResult === "exporter-missing"
      ? "Meta quota exporter units (meta-quota-export.service/.timer) are missing from ~/.config/systemd/user and no .backup copies exist; Meta measurement has no probe. Restore the units to re-arm the Meta session keeper."
      : doc?.status === "session-expired"
        ? `dev.meta.ai session is logged out in the laptop Helium profile; Meta re-login needs a human step (Meta emails a login code) in that profile. Meta measurement is dark${darkSince ? ` since last good ${darkSince}` : ""} until then.`
        : `Meta quota exporter is not producing fresh readings (status=${doc?.status ?? "no document"}, recovery=${recoveryResult}); check meta-quota-export.timer and the laptop Helium dev.meta.ai session.`;
  return finish({
    status: "attention",
    exporterStatus: doc?.status ?? null,
    lastGoodAt: darkSince,
    recovery: { attempted: true, result: recoveryResult },
    alert,
  });
}
