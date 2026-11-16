// Build health panel data (#9924, split from #1239): PR CI lane state
// and the id E2E nightly trend for developing-today/code, generated
// from the GitHub Actions API at view time. The lane set below is
// configuration (which workflows gate id work — the issue's own
// examples: the id workflow and the fleet-rust-tests lane); every
// VALUE the panel shows comes from an Actions API response. Nothing
// is persisted: the live source caches one snapshot for 5 minutes,
// mirroring the #9922 body feed in phase12.ts.
//
// Everything except the fetch functions is pure: the runs parser,
// the lane projection, and the nightly trend projection are tested
// against fixtures, including a failing-nightly case.
export const BUILD_HEALTH_REPO = "developing-today/code";
/** A fetched snapshot older than this renders with the stale warning. */
export const BUILD_HEALTH_STALE_MS = 15 * 60_000;
export const BUILD_HEALTH_RUNS_PER_LANE = 20;
export const BUILD_HEALTH_TREND_RUNS = 14;

export interface BuildHealthLane {
  /** Workflow file under .github/workflows/. */
  file: string;
  /** Display label (the workflow's own name, kept beside the file so
   * a rename on either side is visible instead of silent). */
  label: string;
}

export const BUILD_HEALTH_LANES: BuildHealthLane[] = [
  { file: "id-check.yml", label: "id" },
  { file: "fleet-rust-tests.yml", label: "fleet-rust-tests" },
  { file: "id-hosted-worker-tests.yml", label: "id hosted-worker tests" },
];
export const BUILD_HEALTH_NIGHTLY: BuildHealthLane = { file: "id-e2e-nightly.yml", label: "id e2e nightly" };

/** One Actions run, reduced to what the panel renders. */
export interface ActionsRun {
  id: number;
  status: string;
  conclusion: string | null;
  createdAt: string;
  url: string;
  event: string;
}

export interface LaneRuns {
  lane: BuildHealthLane;
  runs: ActionsRun[];
  /** Why this lane's read failed, when it did; the panel renders the
   * lane as unavailable instead of inventing a state. */
  error: string | null;
}

export interface BuildHealthSnapshot {
  fetchedAt: string;
  lanes: LaneRuns[];
  nightly: LaneRuns;
}

/** Parse one workflow-runs list response. Malformed entries are
 * dropped (a bad row must not cost the lane); a response without a
 * workflow_runs array throws, so the caller records the lane error. */
export function parseWorkflowRuns(json: unknown): ActionsRun[] {
  if (!json || typeof json !== "object") throw new Error("actions runs response is not an object");
  const list = (json as { workflow_runs?: unknown }).workflow_runs;
  if (!Array.isArray(list)) throw new Error("actions runs response has no workflow_runs array");
  const out: ActionsRun[] = [];
  for (const raw of list) {
    if (!raw || typeof raw !== "object") continue;
    const r = raw as Record<string, unknown>;
    if (
      typeof r.id !== "number" ||
      typeof r.status !== "string" ||
      !(typeof r.conclusion === "string" || r.conclusion === null) ||
      typeof r.created_at !== "string" ||
      typeof r.html_url !== "string" ||
      typeof r.event !== "string"
    ) {
      continue;
    }
    out.push({
      id: r.id,
      status: r.status,
      conclusion: r.conclusion,
      createdAt: r.created_at,
      url: r.html_url,
      event: r.event,
    });
  }
  return out;
}

/** Fetch one workflow's recent runs via the Actions REST API.
 * Throws on HTTP errors (never logs the token). */
export async function fetchWorkflowRuns(
  token: string | null,
  fetchFn: typeof fetch,
  file: string,
  perPage: number = BUILD_HEALTH_RUNS_PER_LANE,
): Promise<ActionsRun[]> {
  const url = `https://api.github.com/repos/${BUILD_HEALTH_REPO}/actions/workflows/${encodeURIComponent(file)}/runs?per_page=${perPage}`;
  const res = await fetchFn(url, {
    method: "GET",
    headers: {
      Accept: "application/vnd.github+json",
      "X-GitHub-Api-Version": "2022-11-28",
      "User-Agent": "pm-aggregator-build-health",
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
    },
    signal: AbortSignal.timeout(20_000),
  });
  if (!res.ok) throw new Error(`github Actions HTTP ${res.status}`);
  return parseWorkflowRuns(await res.json());
}

/** Fetch every lane plus the nightly. Per-lane failures are recorded
 * on the lane (error), never thrown: one dead workflow read must not
 * blank the whole panel. */
export async function fetchBuildHealthSnapshot(
  token: string | null,
  fetchFn: typeof fetch = fetch,
  now: () => Date = () => new Date(),
): Promise<BuildHealthSnapshot> {
  const read = async (lane: BuildHealthLane): Promise<LaneRuns> => {
    try {
      return { lane, runs: await fetchWorkflowRuns(token, fetchFn, lane.file), error: null };
    } catch (e) {
      return { lane, runs: [], error: e instanceof Error ? e.message : "unknown error" };
    }
  };
  const lanes = await Promise.all(BUILD_HEALTH_LANES.map(read));
  const nightly = await read(BUILD_HEALTH_NIGHTLY);
  return { fetchedAt: now().toISOString(), lanes, nightly };
}

/** The lane's current state: its newest run, plus the recent
 * completed history (newest first) the panel renders as the trend
 * strip. */
export interface LaneProjection {
  latest: ActionsRun | null;
  history: ActionsRun[];
}

export function projectLane(laneRuns: LaneRuns, historyCount: number = 10): LaneProjection {
  const runs = [...laneRuns.runs].sort((a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt));
  return {
    latest: runs[0] ?? null,
    history: runs.filter((r) => r.status === "completed").slice(0, historyCount),
  };
}

/** The nightly trend: completed runs only, oldest first so the panel
 * reads left-to-right through time, capped at the most recent N. A
 * failing run stays in the trend with its URL — the failure link is
 * the point of the panel. */
export function projectNightlyTrend(nightly: LaneRuns, count: number = BUILD_HEALTH_TREND_RUNS): ActionsRun[] {
  return [...nightly.runs]
    .filter((r) => r.status === "completed")
    .sort((a, b) => Date.parse(a.createdAt) - Date.parse(b.createdAt))
    .slice(-count);
}

/** Freshness for the panel: stale when the fetch is older than
 * BUILD_HEALTH_STALE_MS (an unparseable stamp is stale, never fresh). */
export function buildHealthIsStale(snapshot: BuildHealthSnapshot, now: Date = new Date()): boolean {
  const fetched = Date.parse(snapshot.fetchedAt);
  if (!Number.isFinite(fetched)) return true;
  return now.getTime() - fetched > BUILD_HEALTH_STALE_MS;
}
