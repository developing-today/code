/**
 * Session environment, the v2 replacement for v1's per-command shell.env
 * hook (design doc §3). Two verified facts shape this module:
 *
 *  1. The route is PUT /api/session/:sessionID/environment with payload
 *     { variables: Record<string, string> } and a 204 success.
 *  2. It REPLACES the session environment — core falls back to process.env
 *     only when no map is set. So the adapter sends the full environment
 *     ({ ...baseEnv, ...vars }, stringified), never just the MCPX_* vars:
 *     a partial send would strip PATH/HOME from every shell command in the
 *     session.
 *
 * With no serverUrl (embedded hosts) there is no route; the module degrades
 * quietly to "unknown session", exactly as v1 degrades.
 */
export type FetchLike = (
  url: string,
  init: { method: string; headers?: Record<string, string>; body?: string },
) => Promise<{ ok: boolean; status: number }>;

export type SessionEnvAdapter = {
  /** Send the session's environment once; later calls for the same session are no-ops. */
  ensure(sessionID: string, vars: Record<string, string>): Promise<boolean>;
  /** Sessions whose environment has been sent (or attempted and failed). */
  seen(sessionID: string): boolean;
};

export function createSessionEnv(deps: {
  serverUrl: string | undefined;
  baseEnv: Record<string, string | undefined>;
  fetchFn?: FetchLike;
}): SessionEnvAdapter {
  const done = new Set<string>();
  const fetchFn: FetchLike =
    deps.fetchFn ?? ((url, init) => fetch(url, init) as ReturnType<FetchLike>);
  return {
    seen: (sessionID) => done.has(sessionID),
    async ensure(sessionID, vars) {
      if (done.has(sessionID)) return false;
      // Mark first: a failed send must not be retried on every hook firing.
      done.add(sessionID);
      if (deps.serverUrl === undefined) return false;
      const variables: Record<string, string> = {};
      for (const [k, v] of Object.entries(deps.baseEnv)) {
        if (v !== undefined) variables[k] = v;
      }
      for (const [k, v] of Object.entries(vars)) variables[k] = v;
      const base = deps.serverUrl.replace(/\/+$/, "");
      const res = await fetchFn(
        `${base}/api/session/${encodeURIComponent(sessionID)}/environment`,
        {
          method: "PUT",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ variables }),
        },
      );
      return res.ok;
    },
  };
}
