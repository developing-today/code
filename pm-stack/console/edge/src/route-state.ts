// Per-surface route state for the PM front door (#14944, CF-4;
// design doc/2026-11-24T06-06-29Z_design_pm-cloudflare-migration §5.1,
// as amended by Drew's 2026-11-24 rulings).
//
// The front door holds, for each PM surface, which tier is declared to
// serve it: `hosted` (the Cloudflare-hosted edition) or `tunnel` (the
// laptop edition behind the Cloudflare Tunnel). Route state is data in
// the edge's own storage (rebuildable only — never a source of truth
// for anything else), and it is visible on /console/health per surface:
// a flip, and any automatic fall, can be seen from outside instead of
// being a silent setting buried in a dashboard.
//
// Rules from the design, kept here in one place so every caller goes
// by the same ones:
//
// - A human (or the deploy pipeline) flips a surface's declared tier.
//   A flip is the rollback for a bad hosted deploy and the deliberate
//   switch during a migration wave; it clears any fall, because the
//   surface is now where it was put on purpose.
// - Reads fall automatically (amendment D2): when a hosted read fails,
//   the surface is marked degraded and its reads are served by the
//   tunnel tier until a hosted read succeeds again. The fall is a
//   fact in the state (`degraded`, `fellAt`), never silent.
// - Writes never auto-fall: only GET and HEAD may fall. A write whose
//   hosted path is down is an honest error at that surface, never a
//   silent re-route to another authority.
// - A surface with no route state has no declared tier: callers must
//   not invent one. (Today every surface is tunnel-served by the host
//   behind the front door; a surface gains route state when its hosted
//   edition exists to be flipped to.)

/** The tier a surface is declared to be served by. */
export type RouteTier = "hosted" | "tunnel";

/** One surface's route state, as kept and as health says it. */
export interface SurfaceRoute {
  /** The declared tier: where the surface was put, by a flip. */
  tier: RouteTier;
  /** A hosted surface whose reads are currently falling to the
   * tunnel tier. Only ever true when `tier` is `hosted`. */
  degraded: boolean;
  /** When the current fall began, or null when not fallen. */
  fellAt: number | null;
}

/** Route state for every surface that has one, by surface name. */
export type RouteState = Record<string, SurfaceRoute>;

/** A surface name: a short identifier with no whitespace. Names are
 * the surface identifiers of the design's route table (for example
 * `board`, `health`, `pages`); they are data, not paths. */
export const SURFACE_NAME = /^[^\s/][^\s]{0,63}$/;

const isTier = (value: unknown): value is RouteTier => value === "hosted" || value === "tunnel";

/** Read route state from what storage holds. Anything that does not
 * read as route state is not route state: an entry with an unknown
 * tier, a fall with no time, or a degraded tunnel surface (a fall is
 * hosted → tunnel, so a tunnel surface cannot be fallen) is dropped,
 * never repaired into a guess. */
export function routeState(value: unknown): RouteState {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return {};
  const out: RouteState = {};
  for (const [surface, entry] of Object.entries(value as Record<string, unknown>)) {
    if (!SURFACE_NAME.test(surface)) continue;
    if (entry === null || typeof entry !== "object" || Array.isArray(entry)) continue;
    const { tier, degraded, fellAt } = entry as Partial<SurfaceRoute>;
    if (!isTier(tier)) continue;
    if (degraded !== undefined && typeof degraded !== "boolean") continue;
    if (fellAt !== undefined && fellAt !== null && (typeof fellAt !== "number" || !Number.isFinite(fellAt))) continue;
    const fallen = degraded === true && tier === "hosted";
    out[surface] = { tier, degraded: fallen, fellAt: fallen ? (fellAt ?? null) : null };
  }
  return out;
}

/** The tier actually serving a surface now: a fallen hosted surface
 * is served by the tunnel; anything else by its declared tier. A
 * surface with no state has no serving tier (null), not a default. */
export function servingTier(entry: SurfaceRoute | undefined): RouteTier | null {
  if (!entry) return null;
  return entry.degraded ? "tunnel" : entry.tier;
}

/** Flip a surface to a declared tier (the human / deploy-pipeline
 * operation). The flip clears any fall. Returns the new state; the
 * given state is not changed. An unknown surface name or tier is a
 * caller error and throws, rather than writing state health cannot
 * say. */
export function flipRoute(state: RouteState, surface: string, tier: RouteTier): RouteState {
  if (typeof surface !== "string" || !SURFACE_NAME.test(surface))
    throw new Error(`route state: ${JSON.stringify(surface)} is not a surface name`);
  if (!isTier(tier)) throw new Error(`route state: ${JSON.stringify(tier)} is not a tier`);
  return { ...state, [surface]: { tier, degraded: false, fellAt: null } };
}

/** Whether a request of this method may fall automatically: reads
 * only (GET, HEAD). Writes never auto-fall. */
export function mayAutoFall(method: string): boolean {
  return method === "GET" || method === "HEAD";
}

/** Record that a hosted read of a surface failed: the surface falls
 * to the tunnel tier, visibly, from `at`. Returns the new state and
 * whether this failure is the one that fell (a surface already fallen,
 * a tunnel surface, a surface with no state, and any non-read method
 * do not fall — for a write this is the design's hard rule, not a
 * missing case). */
export function recordReadFailure(
  state: RouteState,
  surface: string,
  method: string,
  at: number,
): { state: RouteState; fell: boolean } {
  const entry = state[surface];
  if (!entry || entry.tier !== "hosted" || entry.degraded || !mayAutoFall(method)) return { state, fell: false };
  return { state: { ...state, [surface]: { tier: entry.tier, degraded: true, fellAt: at } }, fell: true };
}

/** Record that a hosted read of a surface succeeded: a fallen surface
 * stands back up. Returns the new state and whether it had been
 * fallen. */
export function recordHostedSuccess(state: RouteState, surface: string): { state: RouteState; recovered: boolean } {
  const entry = state[surface];
  if (!entry || !entry.degraded) return { state, recovered: false };
  return { state: { ...state, [surface]: { tier: entry.tier, degraded: false, fellAt: null } }, recovered: true };
}

/** What /console/health says of the route state: every surface that
 * has state, by name in order, with its declared tier, the tier
 * actually serving it, and its fall. Nothing else is said — route
 * state holds no secrets and no content. */
export function healthRoutes(state: RouteState): Record<string, SurfaceRoute & { serving: RouteTier }> {
  const out: Record<string, SurfaceRoute & { serving: RouteTier }> = {};
  for (const surface of Object.keys(state).sort()) {
    const entry = state[surface];
    out[surface] = { ...entry, serving: servingTier(entry) as RouteTier };
  }
  return out;
}
