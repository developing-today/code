// The local-daemon transport for the home-page example (#1876 stage 3,
// "Run on my machine").
//
// Stage 2's fake daemon (fake-daemon.js) patches fetch and answers the
// generated client's /v1 routes from in-page mocks. This is the same
// seam with the other end left real: /v1 requests are rewritten to the
// reader's own daemon address and go through the page's genuine fetch,
// so the example runs against the daemon on the reader's machine,
// through the same real generated client. Everything that is not a
// /v1 route passes through untouched.
//
// The daemon side of this is deliberately thin (the recorded choice on
// #1876/#1890): the daemon already refuses browser origins that are
// not in transport.allowedOrigins, and answers CORS for the ones that
// are (internal/daemon/cors.go). Nothing is allow-listed by default,
// so a reader opts in twice -- this control in the page, and their
// daemon's allow-list -- before a docs page can call their daemon.
//
// Two daemon settings shape the failure modes, and the no-daemon
// message names both: daemon.port defaults to a free port reported
// only through the daemon's own health operation, so the page can
// only find a daemon that was given a fixed port; and a daemon with
// transport.bearerToken set cannot be used from a page at all -- a
// browser tab has nowhere safe to keep the token.
//
// installLocalDaemon(base) returns a restore function. Installing
// twice is a no-op that returns the existing restore (the fake
// daemon's contract, kept).

export const DEFAULT_DAEMON_BASE = "http://127.0.0.1:7432";

/** The one error a reader should ever see when the local run cannot
 *  start: what was tried, the two settings that decide it, and the
 *  way back to the simulated run. */
export function noDaemonMessage(base) {
  return (
    `no mcpx daemon answered at ${base}. Is one running? It needs a ` +
    `fixed daemon.port (this address) and this site's origin in ` +
    `transport.allowedOrigins -- and if it requires a bearer token ` +
    `(transport.bearerToken), a browser page cannot run against it. ` +
    `Or turn off "Run on my machine" to use the simulated run.`
  );
}

export function installLocalDaemon(base = DEFAULT_DAEMON_BASE) {
  if (globalThis.__mcpxLocalDaemon) return globalThis.__mcpxLocalDaemon.restore;

  const original = globalThis.fetch.bind(globalThis);
  const root = String(base).replace(/\/+$/, "");

  const patched = async (input, init) => {
    const raw = typeof input === "string" ? input : input.url;
    let url;
    try {
      url = new URL(raw, "http://mcpx.invalid");
    } catch {
      return original(input, init);
    }
    if (!url.pathname.startsWith("/v1/")) return original(input, init);
    try {
      return await original(root + url.pathname + url.search, init);
    } catch {
      // A refused connection and a CORS refusal both surface as a
      // bare TypeError from fetch; the reader cannot act on either
      // raw, so both become the one clear message.
      throw new Error(noDaemonMessage(root));
    }
  };

  const restore = () => {
    if (globalThis.fetch === patched) globalThis.fetch = original;
    delete globalThis.__mcpxLocalDaemon;
  };

  globalThis.fetch = patched;
  globalThis.__mcpxLocalDaemon = { restore };
  return restore;
}
