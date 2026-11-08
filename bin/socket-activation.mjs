// systemd socket activation (the LISTEN_FDS protocol), shared by the
// provider proxies (#1821). When systemd starts a service on behalf of a
// .socket unit it hands the already-listening socket to the process as
// fd 3 (SD_LISTEN_FDS_START) and sets LISTEN_PID / LISTEN_FDS in the
// environment. A socket-activated proxy must serve on that descriptor
// instead of binding its port itself: systemd keeps the listener across
// service restarts, so connections made during a restart queue for the
// next process instead of being refused. A manual launch has no
// LISTEN_FDS and binds the configured port exactly as before.
export const SD_LISTEN_FDS_START = 3;

/** The systemd-passed listening fd for this process, or null. */
export function systemdActivationFd(env = process.env, pid = process.pid) {
  const fds = Number(env.LISTEN_FDS);
  if (!Number.isSafeInteger(fds) || fds < 1) return null;
  // LISTEN_PID, when present, names the process the descriptors were
  // passed to; a mismatch means the environment leaked in from another
  // activation and the fd does not belong to us.
  if (env.LISTEN_PID !== undefined && Number(env.LISTEN_PID) !== pid) return null;
  return SD_LISTEN_FDS_START;
}

/**
 * The `server.listen(...)` target for a proxy: `{ fd }` under systemd
 * socket activation (the socket unit is configured with the same port,
 * so the published endpoint is unchanged), else `{ port, host }` for a
 * direct bind.
 */
export function listenTarget({ port, host }, env = process.env, pid = process.pid) {
  const fd = systemdActivationFd(env, pid);
  return fd === null ? { port, host } : { fd };
}
