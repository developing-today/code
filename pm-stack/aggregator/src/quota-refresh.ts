// Lazy quota refresh (#1171): a page/API load whose newest measurement is
// older than five minutes asks the existing gatherer to run again, without
// blocking the response on provider calls. Concurrent loads collapse to one
// gatherer run (single-flight), and a short cooldown keeps a burst of loads
// from re-triggering once that run has finished.
//
// The trigger reuses the deployed gatherer unit (`quota-gatherer.service`)
// rather than fetching providers here — provider-fetch logic lives only in
// scripts/gather-quota.ts.

import type { QuotaView } from "./model.ts";

/** The issue's freshness bound: five minutes ("past minute or 5 minutes"). */
export const STALE_AFTER_MS = 5 * 60_000;

/** After a run finishes, another trigger is suppressed for this long so a
 *  burst of page loads cannot start a second gatherer run back to back. */
export const TRIGGER_COOLDOWN_MS = 60_000;

export interface RefreshStatus {
  /** Newest measurement was older than `maxAgeMs` (or absent). */
  stale: boolean;
  /** A gatherer run is in flight, or one was requested moments ago. */
  refreshing: boolean;
  /** This call started the gatherer. */
  triggered: boolean;
  /** ISO time of the most recent trigger, or null. */
  lastTriggerAt: string | null;
}

export type RefreshTrigger = (reason: "stale" | "manual") => void | Promise<void>;

/** Newest `measuredAt`/gatherer tick in the view, in epoch ms, or null. */
export function newestMeasurement(view: QuotaView, nowMs = Date.now()): number | null {
  const times: number[] = [];
  const add = (raw: string | null | undefined) => {
    if (!raw) return;
    const t = Date.parse(raw);
    // Future timestamps (clock skew) count as fresh, not as very stale.
    if (!Number.isNaN(t)) times.push(Math.min(t, nowMs));
  };
  for (const pool of view.pools ?? []) {
    add(pool.freshness?.measuredAt);
    for (const credential of pool.credentials ?? []) add(credential.freshness?.measuredAt);
  }
  if (view.gatherer) {
    add(view.gatherer.generatedAt);
    add(view.gatherer.tickAt);
    for (const provider of view.gatherer.providers ?? []) add(provider.measuredAt);
  }
  return times.length ? Math.max(...times) : null;
}

/** True when the data is missing or older than `maxAgeMs`. */
export function isQuotaStale(view: QuotaView, nowMs = Date.now(), maxAgeMs = STALE_AFTER_MS): boolean {
  const newest = newestMeasurement(view, nowMs);
  return newest === null || nowMs - newest > maxAgeMs;
}

export interface QuotaRefresher {
  /** Trigger when stale; safe to call on every load. */
  request(view: QuotaView, nowMs?: number): RefreshStatus;
  /** Trigger regardless of freshness (manual refresh-now control). */
  manual(view: QuotaView, nowMs?: number): RefreshStatus;
  /** Test/introspection hook: is a run in flight? */
  refreshing(): boolean;
}

export function createQuotaRefresher(opts: {
  trigger: RefreshTrigger;
  now?: () => number;
  maxAgeMs?: number;
  cooldownMs?: number;
}): QuotaRefresher {
  const now = opts.now ?? (() => Date.now());
  const maxAgeMs = opts.maxAgeMs ?? STALE_AFTER_MS;
  const cooldownMs = opts.cooldownMs ?? TRIGGER_COOLDOWN_MS;
  let inFlight: Promise<void> | null = null;
  let lastTriggerAt: number | null = null;

  const iso = () => (lastTriggerAt === null ? null : new Date(lastTriggerAt).toISOString());
  const suppressed = (stale: boolean): RefreshStatus => ({
    stale,
    refreshing: true,
    triggered: false,
    lastTriggerAt: iso(),
  });

  function kick(reason: "stale" | "manual"): void {
    lastTriggerAt = now();
    const fail = (err: unknown) => {
      // Fail soft: a refresh that cannot start must not break the page.
      console.error(`aggregator: quota refresh trigger failed (${reason}):`, err);
    };
    try {
      const result = opts.trigger(reason);
      const run = Promise.resolve(result)
        .catch(fail)
        .finally(() => {
          if (inFlight === run) inFlight = null;
        });
      inFlight = run;
    } catch (err) {
      // Synchronous failure: nothing is in flight.
      fail(err);
    }
  }

  return {
    request(view, nowMs = now()) {
      const stale = isQuotaStale(view, nowMs, maxAgeMs);
      if (!stale) return { stale: false, refreshing: false, triggered: false, lastTriggerAt: iso() };
      if (inFlight) return suppressed(true);
      if (lastTriggerAt !== null && nowMs - lastTriggerAt < cooldownMs) return suppressed(true);
      kick("stale");
      return { stale: true, refreshing: true, triggered: true, lastTriggerAt: iso() };
    },
    manual(view, nowMs = now()) {
      const stale = isQuotaStale(view, nowMs, maxAgeMs);
      if (inFlight) return suppressed(stale);
      kick("manual");
      return { stale, refreshing: true, triggered: true, lastTriggerAt: iso() };
    },
    refreshing: () => inFlight !== null,
  };
}

/**
 * Trigger by asking systemd to start the deployed gatherer unit. The unit is
 * the same `quota-gatherer.service` the 15-minute timer runs, so there is one
 * implementation of provider fetching. `QUOTA_REFRESH_CMD` overrides the
 * command (space-split) for hosts without a user systemd session.
 */
export function systemdTrigger(command = process.env.QUOTA_REFRESH_CMD ?? "systemctl --user start --no-block quota-gatherer.service"): RefreshTrigger {
  return async () => {
    const argv = command.trim().split(/\s+/);
    if (argv.length === 0 || !argv[0]) throw new Error("empty refresh command");
    const proc = Bun.spawn(argv, { stdin: "ignore", stdout: "ignore", stderr: "ignore" });
    // Do not keep the request alive on the child; systemd does the work.
    proc.unref();
  };
}
