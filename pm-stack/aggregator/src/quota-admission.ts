/** #2732 packet A: normalized, pure projection only. No production adapter or
 * admission consumer. Callers must supply exact billing scopes and ALL binding
 * windows; dashboard pool/service labels are deliberately not accepted as identity.
 * Times are epoch milliseconds. Prior walls/stops must be carried by the caller;
 * omission is NOT a release protocol. This module neither fetches nor persists.
 *
 * Stage-3 S4 (#12698, design D4): the caller may also carry forecast rows
 * (the S1 projection, per exact billing scope like observations). A fresh
 * forecast projected from the window's current reading that says projected
 * + wallsBeforeReset, with the projected wall at least the window's
 * configured forecastMarginMs before the reset, adds pace_hold to an
 * otherwise eligible window — and can do nothing else: it never overrides
 * a measured verdict, never manufactures evidence for an unknown window,
 * and a stale, superseded, conflicting or honesty-state forecast is
 * exactly current behaviour (#12698 criterion 3). Pool-level consumption
 * exclusions from S2's calibration (e.g. the bare opencode-go monthly
 * series) are the caller's duty: display pool identity is not accepted
 * here, so the caller assembles rows only for scopes it may consume.
 */
export type Verdict = "eligible" | "unknown" | "pace_hold" | "walled" | "stopped";
export interface Scope {
  provider: string;
  account: string;
  bucket: string;
}
export interface Binding extends Scope {
  /** Stable credential aliases only; never credentials or display labels. */
  aliases: readonly string[];
  windows: readonly {
    kind: string;
    period: string;
    resetsAt: number;
    freshnessMs: number;
    collectionMs: number;
    reservePct?: number;
    pacePpHour?: number;
    /** S4: a projected wall at least this many ms before the reset
     * yields pace_hold. Absent = forecasts never affect this window
     * (per-window opt-in; the one live pool's windows carry it). */
    forecastMarginMs?: number;
  }[];
}
export interface Observation extends Scope {
  kind: string;
  period: string;
  id: string;
  source: string;
  measuredAt: number;
  resetsAt: number;
  percent: number;
  state: "ok" | "error";
  retained?: boolean;
  /** Stable allowance identity; a changed denominator breaks pace continuity. */
  denominator: string;
}
export interface Stop extends Scope {
  id: string;
  effectiveAt: number;
  reason: string;
  /** Explicit release evidence supplied by the owner, not inferred from quota. */
  releasedAt?: number;
}
export interface PriorWall extends Scope {
  kind: string;
  period: string;
  resetsAt: number;
  evidenceId: string;
}
export interface Forecast extends Scope {
  kind: string;
  period: string;
  /** Evidence id of the forecast row (the S1 product/ledger row). */
  id: string;
  /** The underlying reading's stamp the projection was made from. */
  measuredAt: number;
  resetsAt: number;
  /** The S1 projection state, verbatim (quota-analytics ProjectionState). */
  state: "projected" | "walled" | "no-wall-before-reset" | "insufficient-history";
  /** Projected instant the window reaches 100%, epoch ms. */
  projectedWallAt: number | null;
  wallsBeforeReset: boolean | null;
}
export interface AdmissionInput {
  generation: string;
  bindings: readonly Binding[];
  observations: readonly Observation[];
  stops: readonly Stop[];
  priorWalls: readonly PriorWall[];
  /** S4 forecast rows, per exact scope; absent = pre-S4 behaviour. */
  forecasts?: readonly Forecast[];
}
export interface WindowDecision {
  kind: string;
  period: string;
  verdict: Verdict;
  reasons: string[];
  evidenceIds: string[];
  measuredAt: number | null;
  resetsAt: number;
  pace: "unknown" | "available";
  ratePpHour: number | null;
  source: string | null;
  percent: number | null;
}
export interface AccountDecision extends Scope {
  aliases: string[];
  verdict: Verdict;
  reasons: string[];
  windows: WindowDecision[];
  activeStops: { id: string; effectiveAt: number; reason: string }[];
}
export interface AdmissionProjection {
  version: 1;
  generation: string;
  evaluatedAt: number;
  accounts: AccountDecision[];
  seats: { provider: string; alias: string; account: string | null; buckets: string[]; verdict: Verdict }[];
}
const ordinal = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);
const rank: Record<Verdict, number> = { eligible: 0, unknown: 1, pace_hold: 2, walled: 3, stopped: 4 };
const nonempty = (s: string) => typeof s === "string" && s.trim().length > 0;
const finite = (n: number) => Number.isFinite(n);
const key = (s: Scope) => JSON.stringify([s.provider, s.account, s.bucket]);
const same = (a: Scope, b: Scope) => key(a) === key(b);
const validScope = (s: Scope) => [s.provider, s.account, s.bucket].every(nonempty);
const sorted = (s: readonly string[]) => [...new Set(s)].sort();
const worst = (v: Verdict[]) => v.reduce((a, b) => (rank[a] >= rank[b] ? a : b), "eligible");

/** The one forecast row that may speak for this window, or null. A row
 * speaks only when it is the latest, internally consistent, projected
 * from the window's current reading, and still fresh by the same bound
 * the measured path applies to observations; every other row — stale,
 * superseded, conflicting, or an honesty state — is current behaviour
 * (#12698 criterion 3), never a reason and never a hold. */
function applicableForecast(
  input: AdmissionInput,
  b: Binding,
  w: Binding["windows"][number],
  now: number,
  latestMeasuredAt: number | null,
): Forecast | null {
  const rows = (input.forecasts ?? []).filter(
    (x) => same(x, b) && x.kind === w.kind && x.period === w.period && x.resetsAt === w.resetsAt,
  );
  if (!rows.length) return null;
  const ids = new Map<string, string>();
  for (const x of rows) {
    if (
      !nonempty(x.id) ||
      !finite(x.measuredAt) ||
      x.measuredAt > now ||
      (x.projectedWallAt !== null && !finite(x.projectedWallAt)) ||
      (x.wallsBeforeReset !== null && typeof x.wallsBeforeReset !== "boolean") ||
      (x.state !== "projected" &&
        x.state !== "walled" &&
        x.state !== "no-wall-before-reset" &&
        x.state !== "insufficient-history")
    )
      return null;
    const signature = JSON.stringify([x.measuredAt, x.state, x.projectedWallAt, x.wallsBeforeReset]);
    if (ids.has(x.id) && ids.get(x.id) !== signature) return null;
    ids.set(x.id, signature);
  }
  const unique = [...new Map(rows.map((x) => [x.id, x])).values()].sort(
    (a, c) => c.measuredAt - a.measuredAt || ordinal(a.id, c.id),
  );
  const latest = unique[0];
  const tied = unique.filter((x) => x.measuredAt === latest.measuredAt);
  if (
    tied.some(
      (x) =>
        x.state !== latest.state ||
        x.projectedWallAt !== latest.projectedWallAt ||
        x.wallsBeforeReset !== latest.wallsBeforeReset,
    )
  )
    return null;
  if (latest.state !== "projected" || latest.wallsBeforeReset !== true || latest.projectedWallAt === null) return null;
  if (latest.projectedWallAt >= w.resetsAt) return null;
  if (now - latest.measuredAt > Math.min(w.freshnessMs, 60 * 60_000)) return null;
  if (latestMeasuredAt === null || latest.measuredAt < latestMeasuredAt) return null;
  return latest;
}

function projectWindow(input: AdmissionInput, b: Binding, w: Binding["windows"][number], now: number): WindowDecision {
  const out: WindowDecision = {
    kind: w.kind,
    period: w.period,
    verdict: "unknown",
    reasons: [],
    evidenceIds: [],
    measuredAt: null,
    resetsAt: w.resetsAt,
    pace: "unknown",
    ratePpHour: null,
    source: null,
    percent: null,
  };
  const unknown = (reason: string) => {
    out.reasons.push(reason);
    return out;
  };
  if (
    !nonempty(w.kind) ||
    !nonempty(w.period) ||
    !finite(w.resetsAt) ||
    !finite(w.freshnessMs) ||
    w.freshnessMs <= 0 ||
    !finite(w.collectionMs) ||
    w.collectionMs <= 0 ||
    (w.reservePct !== undefined && (!finite(w.reservePct) || w.reservePct < 0 || w.reservePct > 100)) ||
    (w.pacePpHour !== undefined && (!finite(w.pacePpHour) || w.pacePpHour < 0)) ||
    (w.forecastMarginMs !== undefined && (!finite(w.forecastMarginMs) || w.forecastMarginMs < 0))
  )
    return unknown("invalid_window_policy");
  if (w.resetsAt <= now) return unknown("period_expired");
  const wall = input.priorWalls.filter(
    (x) =>
      same(x, b) && x.kind === w.kind && x.period === w.period && x.resetsAt === w.resetsAt && nonempty(x.evidenceId),
  );
  if (wall.length) {
    out.verdict = "walled";
    out.reasons = ["prior_wall_same_period"];
    out.evidenceIds = sorted(wall.map((x) => x.evidenceId));
    return out;
  }
  const rows = input.observations.filter((x) => same(x, b) && x.kind === w.kind && x.period === w.period);
  if (!rows.length) return unknown("missing_observation");
  // A corrupt member cannot silently vanish and turn a bucket healthy. Other
  // scopes are unaffected. Duplicate observation IDs must be byte-equivalent
  // in normalized semantics; re-reading a cached observation adds no evidence.
  const ids = new Map<string, string>();
  for (const x of rows) {
    if (
      !nonempty(x.id) ||
      !nonempty(x.source) ||
      !nonempty(x.denominator) ||
      !finite(x.measuredAt) ||
      x.measuredAt > now ||
      !finite(x.percent) ||
      x.percent < 0 ||
      x.percent > 100 ||
      x.resetsAt !== w.resetsAt ||
      x.measuredAt >= x.resetsAt ||
      (x.state !== "ok" && x.state !== "error")
    )
      return unknown("invalid_observation");
    const signature = JSON.stringify([
      x.source,
      x.measuredAt,
      x.resetsAt,
      x.percent,
      x.state,
      !!x.retained,
      x.denominator,
    ]);
    if (ids.has(x.id) && ids.get(x.id) !== signature) return unknown("conflicting_observation_id");
    ids.set(x.id, signature);
  }
  const unique = [...new Map(rows.map((x) => [x.id, x])).values()].sort(
    (a, c) => c.measuredAt - a.measuredAt || ordinal(a.id, c.id),
  );
  const latest = unique[0];
  const tied = unique.filter((x) => x.measuredAt === latest.measuredAt);
  if (
    tied.some(
      (x) =>
        x.percent !== latest.percent ||
        x.state !== latest.state ||
        !!x.retained !== !!latest.retained ||
        x.denominator !== latest.denominator,
    )
  )
    return unknown("conflicting_latest_read");
  out.measuredAt = latest.measuredAt;
  out.source = latest.source;
  out.percent = latest.percent;
  out.evidenceIds = sorted(tied.map((x) => x.id));
  if (latest.state !== "ok" || latest.retained) return unknown("unavailable_observation");
  if (now - latest.measuredAt > Math.min(w.freshnessMs, 60 * 60_000)) return unknown("stale_observation");
  const previous = unique.find((x) => x.measuredAt < latest.measuredAt);
  // Require adjacent distinct observations. A contradictory intervening read
  // breaks sustained evidence; it must not be skipped to find a convenient high.
  const previousTies = previous ? unique.filter((x) => x.measuredAt === previous.measuredAt) : [];
  const previousConsistent =
    previous &&
    previousTies.every(
      (x) =>
        x.percent === previous.percent &&
        x.state === previous.state &&
        !!x.retained === !!previous.retained &&
        x.denominator === previous.denominator,
    );
  const continuous =
    previousConsistent && previous.state === "ok" && !previous.retained && previous.denominator === latest.denominator;
  if (
    latest.percent === 100 ||
    (latest.percent >= 99.5 &&
      continuous &&
      previous.percent >= 99.5 &&
      latest.measuredAt - previous.measuredAt >= w.collectionMs)
  ) {
    out.verdict = "walled";
    out.reasons = [latest.percent === 100 ? "known_100" : "sustained_99_5"];
    if (previous && latest.percent !== 100) out.evidenceIds = sorted([...out.evidenceIds, previous.id]);
    return out;
  }
  out.verdict = "eligible";
  if (continuous && previous.percent <= latest.percent) {
    out.pace = "available";
    out.ratePpHour = ((latest.percent - previous.percent) * 3_600_000) / (latest.measuredAt - previous.measuredAt);
  }
  if (w.reservePct !== undefined && latest.percent >= w.reservePct) {
    out.verdict = "pace_hold";
    out.reasons.push("reserve_threshold");
  }
  if (
    w.pacePpHour !== undefined &&
    out.ratePpHour !== null &&
    out.ratePpHour > w.pacePpHour &&
    latest.measuredAt + ((100 - latest.percent) / out.ratePpHour) * 3_600_000 < w.resetsAt
  ) {
    out.verdict = "pace_hold";
    out.reasons.push("projected_exhaustion");
    if (previous) out.evidenceIds = sorted([...out.evidenceIds, previous.id]);
  }
  if (out.verdict === "eligible" && w.forecastMarginMs !== undefined) {
    const f = applicableForecast(input, b, w, now, out.measuredAt);
    if (f && f.projectedWallAt !== null && w.resetsAt - f.projectedWallAt >= w.forecastMarginMs) {
      out.verdict = "pace_hold";
      out.reasons.push("forecast_projected_wall");
      out.evidenceIds = sorted([...out.evidenceIds, f.id]);
    }
  }
  if (out.verdict === "eligible") out.reasons.push("fresh_window");
  return out;
}

export function buildQuotaAdmission(input: AdmissionInput, nowMs: number): AdmissionProjection {
  if (!finite(nowMs) || !nonempty(input.generation)) throw new Error("invalid projection clock or generation");
  // Duplicate bucket declarations and aliases spanning independent accounts are
  // ambiguous configuration, never an opportunity to select the healthiest row.
  const buckets = new Map<string, number>();
  const aliases = new Map<string, Set<string>>();
  for (const b of input.bindings) {
    buckets.set(key(b), (buckets.get(key(b)) ?? 0) + 1);
    for (const a of b.aliases) {
      const ak = JSON.stringify([b.provider, a]);
      if (!aliases.has(ak)) aliases.set(ak, new Set());
      aliases.get(ak)!.add(b.account);
    }
  }
  const accounts = input.bindings
    .map((b) => {
      const out: AccountDecision = {
        provider: b.provider,
        account: b.account,
        bucket: b.bucket,
        aliases: sorted(b.aliases),
        verdict: "unknown",
        reasons: [],
        windows: [],
        activeStops: [],
      };
      const ambiguous =
        buckets.get(key(b)) !== 1 ||
        b.aliases.some((a) => !nonempty(a) || aliases.get(JSON.stringify([b.provider, a]))!.size !== 1);
      if (
        !validScope(b) ||
        !b.aliases.length ||
        !b.windows.length ||
        ambiguous ||
        new Set(b.windows.map((w) => w.kind)).size !== b.windows.length
      )
        out.reasons.push("invalid_binding");
      else {
        out.windows = b.windows.map((w) => projectWindow(input, b, w, nowMs)).sort((a, c) => ordinal(a.kind, c.kind));
        out.verdict = worst(out.windows.map((w) => w.verdict));
        out.reasons = sorted(out.windows.flatMap((w) => w.reasons));
      }
      const stops = input.stops.filter((x) => same(x, b));
      if (
        stops.some(
          (x) =>
            !nonempty(x.id) ||
            !nonempty(x.reason) ||
            !finite(x.effectiveAt) ||
            (x.releasedAt !== undefined && (!finite(x.releasedAt) || x.releasedAt < x.effectiveAt)),
        )
      ) {
        out.verdict = worst([out.verdict, "unknown"]);
        out.reasons.push("invalid_stop");
      }
      const activeStops = stops.filter(
        (x) =>
          nonempty(x.id) &&
          finite(x.effectiveAt) &&
          x.effectiveAt <= nowMs &&
          (x.releasedAt === undefined || !finite(x.releasedAt) || x.releasedAt > nowMs || x.releasedAt < x.effectiveAt),
      );
      if (activeStops.length) {
        out.activeStops = activeStops
          .map((x) => ({ id: x.id, effectiveAt: x.effectiveAt, reason: x.reason }))
          .sort((a, b) => ordinal(JSON.stringify(a), JSON.stringify(b)));
        out.verdict = "stopped";
        out.reasons.push("active_stop");
      }
      out.reasons = sorted(out.reasons);
      return out;
    })
    .sort((a, b) => ordinal(key(a), key(b)) || ordinal(JSON.stringify(a), JSON.stringify(b)));
  const seats = [...aliases.keys()].sort(ordinal).map((ak) => {
    const [provider, alias] = JSON.parse(ak) as [string, string];
    const rows = accounts.filter((a) => a.provider === provider && a.aliases.includes(alias));
    const owners = aliases.get(ak)!;
    return {
      provider,
      alias,
      account: owners.size === 1 ? [...owners][0] : null,
      buckets: sorted(rows.map((a) => a.bucket)),
      verdict: owners.size === 1 ? worst(rows.map((a) => a.verdict)) : ("unknown" as Verdict),
    };
  });
  return { version: 1, generation: input.generation, evaluatedAt: nowMs, accounts, seats };
}
