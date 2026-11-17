// Bound for every outbound fetch dispatch makes on a request path
// (issue #10825): Bun/Node fetch has no default timeout, so a stalled
// Supabase, opencode, or GitHub endpoint would park the handler and its
// worker slot forever. Pass `signal: outboundTimeout()` on each call;
// the resulting TimeoutError propagates through each client's existing
// error mapping (OpenCodeError in opencode.ts, gh fallback in
// github.ts, raw in queue.ts like every other network failure there).
export const OUTBOUND_FETCH_TIMEOUT_MS = 15_000;

export function outboundTimeout(ms = OUTBOUND_FETCH_TIMEOUT_MS): AbortSignal {
  return AbortSignal.timeout(ms);
}
