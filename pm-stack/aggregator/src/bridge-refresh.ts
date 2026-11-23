// Disabled-by-default refresh pilot for the existing queue collector. This
// module owns no credentials, source data, timestamps, files or network client.
export const BRIDGE_REFRESH_INTERVAL_MS = 65_000;
export const BRIDGE_REFRESH_LIFETIME_MS = 3_600_000;
export const BRIDGE_REFRESH_MAX_RUNS = 57;

interface Timer { unref?(): unknown }
interface Options {
  snapshot?: string;
  refresh?: string;
  signal?: AbortSignal;
  now?: () => number;
  schedule?: (callback: () => void, milliseconds: number) => Timer;
  cancel?: (timer: Timer) => void;
}

/** Both literal "1" flags are required; undefined/"0" disable. Unknown values
 * are refused before a timer or collector starts. A process restart begins a
 * new pilot only if both flags remain set: deployment approval must include
 * removing them after the one-hour observation window.
 */
export function startBridgeRefresh(
  collect: () => Promise<unknown>,
  options: Options = {},
) {
  const { snapshot, refresh, signal } = options;
  for (const value of [snapshot, refresh]) {
    if (value !== undefined && value !== "0" && value !== "1") throw new Error("invalid bridge refresh flag");
  }
  const enabled = snapshot === "1" && refresh === "1";
  const now = options.now ?? (() => performance.now());
  const schedule = options.schedule ?? ((callback, ms) => setTimeout(callback, ms));
  const cancel = options.cancel ?? (timer => clearTimeout(timer as ReturnType<typeof setTimeout>));
  let stopped = !enabled || Boolean(signal?.aborted), running = false, runs = 0;
  let next: Timer | undefined, deadline: Timer | undefined;
  let expiresAt = 0;

  const stop = () => {
    stopped = true;
    if (next) cancel(next);
    if (deadline) cancel(deadline);
    next = undefined; deadline = undefined;
    signal?.removeEventListener("abort", stop);
  };
  const arm = (callback: () => void, ms: number) => {
    const timer = schedule(callback, ms);
    timer.unref?.();
    return timer;
  };
  const tick = async () => {
    if (stopped || running) return;
    if (now() >= expiresAt || runs >= BRIDGE_REFRESH_MAX_RUNS) { stop(); return; }
    running = true;
    runs++;
    try { await collect(); }
    catch { /* Existing collector owns failure publication; no raw error logs. */ }
    finally {
      running = false;
      // The independent deadline fires even when collect never settles. A late
      // completion may publish its genuine observation, but cannot rearm us.
      if (!stopped && now() < expiresAt && runs < BRIDGE_REFRESH_MAX_RUNS) {
        next = arm(() => { next = undefined; void tick(); }, BRIDGE_REFRESH_INTERVAL_MS);
      } else stop();
    }
  };
  if (!stopped) {
    const start = now();
    if (!Number.isFinite(start)) throw new Error("invalid bridge refresh clock");
    expiresAt = start + BRIDGE_REFRESH_LIFETIME_MS;
    signal?.addEventListener("abort", stop, { once: true });
    deadline = arm(stop, BRIDGE_REFRESH_LIFETIME_MS);
    void tick();
  }
  return Object.freeze({ enabled, stop, get stopped() { return stopped; }, get running() { return running; }, get runs() { return runs; } });
}
