// A deadline for a request that leaves nothing running behind it.

/** `work`, given `ms` to finish: the signal it is handed aborts then,
 * with the same `TimeoutError` as `AbortSignal.timeout(ms)` gives.
 *
 * `AbortSignal.timeout(ms)` would do, but its timer runs on for the
 * whole of `ms` after the work is long over, and Cloudflare does not
 * put a Durable Object away while a timer of its is running. Measured
 * in the local runtime on 2026-11-24: an object whose last request had
 * asked GitHub one thing (twenty seconds allowed, answered in a few
 * milliseconds) was still in memory 26 seconds later, and one whose
 * last request had asked nothing was gone within 14. An object in
 * memory is an object billed for. Here the timer is cleared as soon as
 * the work settles, however it settles.
 *
 * The work is everything the deadline is for: a caller that reads a
 * response's body reads it inside `work`, since the signal is what
 * stops a body that never ends. */
export async function timed<T>(ms: number, work: (signal: AbortSignal) => Promise<T>): Promise<T> {
  const stop = new AbortController();
  const timer = setTimeout(() => stop.abort(new DOMException("The operation timed out.", "TimeoutError")), ms);
  try {
    return await work(stop.signal);
  } finally {
    clearTimeout(timer);
  }
}

/** A response whose body will not be read is let go of, so that the
 * connection is not kept open for it. (A deadline's signal did this
 * when it ran out, long after; one that is cleared never does.) Given
 * anything else, or a body already read, it does nothing. */
export function unread(response: unknown): void {
  const body = (response as { body?: { cancel?: () => Promise<void> } | null } | null | undefined)?.body;
  try {
    void body?.cancel?.().catch(() => {});
  } catch {
    // A body that cannot be let go of is left to the runtime.
  }
}
