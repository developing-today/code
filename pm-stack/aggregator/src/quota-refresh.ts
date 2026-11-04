// Lazy quota refresh (#1171): a page/API load whose newest measurement is
// older than five minutes asks the existing gatherer to run again, without
// blocking the response on provider calls. Concurrent loads share one request
// and keep a refreshing status until the data advances or the unit timeout.
//
// The trigger reuses the deployed gatherer unit (quota-gatherer.service)
// rather than fetching providers here — provider-fetch logic lives only in
// scripts/gather-quota.ts.

import type { QuotaView } from "./model.ts";

/** The issue's freshness bound: five minutes. */
export const STALE_AFTER_MS = 5 * 60_000;
/** Gatherer systemd TimeoutStartSec is 300s; add one minute of grace. */
export const MAX_REFRESH_MS = 6 * 60_000;
/** Avoid rapid retries after a trigger command fails to start. */
export const TRIGGER_COOLDOWN_MS = 60_000;

export interface RefreshStatus {
  /** Newest measurement was older than maxAgeMs (or absent). */
  stale: boolean;
  /** A gatherer run is expected to be in flight. */
  refreshing: boolean;
  /** This call started the gatherer. */
  triggered: boolean;
  /** ISO time of the most recent trigger, or null. */
  lastTriggerAt: string | null;
}

export type RefreshTrigger = (reason: "stale" | "manual") => void | Promise<void>;

/** Newest non-error provider/pool measurement in the view, in epoch ms, or null. */
export function newestMeasurement(view: QuotaView, nowMs = Date.now()): number | null {
  const times: number[] = [];
  const add = (raw: string | null | undefined, state?: "ok" | "stale" | "error") => {
    if (!raw || state === "error") return;
    const t = Date.parse(raw);
    // Future timestamps (clock skew) count as fresh, not as very stale.
    if (!Number.isNaN(t)) times.push(Math.min(t, nowMs));
  };
  for (const pool of view.pools ?? []) {
    add(pool.freshness?.measuredAt, pool.freshness?.state);
    for (const credential of pool.credentials ?? []) {
      add(credential.freshness?.measuredAt, credential.freshness?.state);
    }
  }
  if (view.gatherer) {
    for (const provider of view.gatherer.providers ?? []) add(provider.measuredAt, provider.state);
  }
  return times.length ? Math.max(...times) : null;
}

/** True when the data is missing or older than maxAgeMs. */
export function isQuotaStale(view: QuotaView, nowMs = Date.now(), maxAgeMs = STALE_AFTER_MS): boolean {
  const newest = newestMeasurement(view, nowMs);
  return newest === null || nowMs - newest > maxAgeMs;
}

export interface QuotaRefresher {
  /** Trigger when stale; safe to call on every load. */
  request(view: QuotaView, nowMs?: number): RefreshStatus;
  /** Trigger regardless of freshness (manual refresh-now control). */
  manual(view: QuotaView, nowMs?: number): RefreshStatus;
  /** Test/introspection hook: is a run expected to be in flight? */
  refreshing(): boolean;
}

export function createQuotaRefresher(opts: {
  trigger: RefreshTrigger;
  now?: () => number;
  maxAgeMs?: number;
  maxRefreshMs?: number;
  cooldownMs?: number;
}): QuotaRefresher {
  const now = opts.now ?? (() => Date.now());
  const maxAgeMs = opts.maxAgeMs ?? STALE_AFTER_MS;
  const maxRefreshMs = opts.maxRefreshMs ?? MAX_REFRESH_MS;
  const cooldownMs = opts.cooldownMs ?? TRIGGER_COOLDOWN_MS;
  // Track the gatherer result, not the short-lived systemctl --no-block child.
  let active: { startedAt: number; baseline: number | null } | null = null;
  let lastTriggerAt: number | null = null;

  const iso = () => (lastTriggerAt === null ? null : new Date(lastTriggerAt).toISOString());
  const status = (stale: boolean, refreshing: boolean, triggered = false): RefreshStatus => ({
    stale,
    refreshing,
    triggered,
    lastTriggerAt: iso(),
  });

  function observe(view: QuotaView, nowMs: number): boolean {
    if (!active) return false;
    const measurement = newestMeasurement(view, nowMs);
    const dataAdvanced = measurement !== null && (active.baseline === null || measurement > active.baseline);
    if (dataAdvanced || nowMs - active.startedAt >= maxRefreshMs) {
      active = null;
      return false;
    }
    return true;
  }

  function kick(reason: "stale" | "manual", view: QuotaView, nowMs: number): void {
    const run = { startedAt: nowMs, baseline: newestMeasurement(view, nowMs) };
    // Set synchronously so concurrent request handlers share the trigger.
    active = run;
    lastTriggerAt = nowMs;
    const fail = (err: unknown) => {
      console.error("aggregator: quota refresh trigger failed (" + reason + "):", err);
      if (active === run) active = null;
    };
    try {
      // The trigger only asks systemd to queue the existing gatherer; it does
      // not wait for provider calls or the gatherer's five-minute run.
      void Promise.resolve(opts.trigger(reason)).catch(fail);
    } catch (err) {
      fail(err);
    }
  }

  return {
    request(view, nowMs = now()) {
      const stale = isQuotaStale(view, nowMs, maxAgeMs);
      const refreshing = observe(view, nowMs);
      if (!stale) return status(false, refreshing);
      if (refreshing) return status(true, true);
      if (lastTriggerAt !== null && nowMs - lastTriggerAt < cooldownMs) return status(true, false);
      kick("stale", view, nowMs);
      return status(true, true, true);
    },
    manual(view, nowMs = now()) {
      const stale = isQuotaStale(view, nowMs, maxAgeMs);
      if (observe(view, nowMs)) return status(stale, true);
      kick("manual", view, nowMs);
      return status(stale, true, true);
    },
    refreshing: () => active !== null,
  };
}

/** Queue the deployed gatherer; provider fetching remains in that service. */
export function systemdTrigger(
  command = process.env.QUOTA_REFRESH_CMD ?? "systemctl --user start --no-block quota-gatherer.service",
): RefreshTrigger {
  return async () => {
    const argv = command.trim().split(/\s+/);
    if (argv.length === 0 || !argv[0]) throw new Error("empty refresh command");
    const proc = Bun.spawn(argv, { stdin: "ignore", stdout: "ignore", stderr: "ignore" });
    const exitCode = await proc.exited;
    if (exitCode !== 0) throw new Error("systemd gatherer start exited with code " + exitCode);
  };
}
