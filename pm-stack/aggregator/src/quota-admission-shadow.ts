/** #2732 B1: evaluate exact route applicability for a read-only shadow check.
 * No registry, provider probes, or queue/session mutation. The caller must
 * supply a complete, independently verified applicability receipt; neither
 * a display pool nor an alias alone proves what a model consumes.
 */
import { buildQuotaAdmission, type AdmissionInput, type Verdict, type WindowDecision } from "./quota-admission.ts";

export interface ShadowRoute {
  provider: string;
  alias: string;
  model: string;
  effort: string;
}
export interface Applicability extends ShadowRoute {
  evidenceId: string;
  /** Exclusive validity bound for the independently verified mapping. */
  verifiedUntil: number;
  account: string;
  /** All metered windows for this precise route; empty is not permission. */
  required: readonly { bucket: string; kind: string; period: string; resetsAt: number }[];
}
export interface SelectedRoute extends ShadowRoute {
  account: string;
}
type Phase = "launch" | "resume";
interface ShadowWindow {
  bucket: string;
  kind: string;
  period: string | null;
  verdict: Verdict;
  reasons: string[];
  evidenceIds: string[];
  measuredAt: number | null;
  resetsAt: number | null;
}
const annotate = (
  bucket: string,
  kind: string,
  verdict: Verdict,
  reasons: string[],
  window?: WindowDecision,
  evidenceIds: string[] = window?.evidenceIds ?? [],
): ShadowWindow => ({
  bucket,
  kind,
  verdict,
  reasons,
  evidenceIds,
  period: window?.period ?? null,
  measuredAt: window?.measuredAt ?? null,
  resetsAt: window?.resetsAt ?? null,
});
const routeKey = (r: ShadowRoute) => JSON.stringify([r.provider, r.alias, r.model, r.effort]);
const nonempty = (s: string) => typeof s === "string" && s.trim() !== "";
const rank: Record<Verdict, number> = { eligible: 0, unknown: 1, pace_hold: 2, walled: 3, stopped: 4 };
const worst = (values: Verdict[]) => values.reduce((a, b) => (rank[a] >= rank[b] ? a : b), "eligible");

export function evaluateAdmissionShadow(
  input: AdmissionInput,
  phase: Phase,
  candidates: readonly ShadowRoute[],
  applicability: readonly Applicability[],
  nowMs: number,
  actual?: SelectedRoute,
) {
  const projection = buildQuotaAdmission(input, nowMs);
  const decisions = candidates.map((candidate) => {
    const route: ShadowRoute = {
      provider: candidate.provider,
      alias: candidate.alias,
      model: candidate.model,
      effort: candidate.effort,
    };
    const key = routeKey(route);
    const unknown = (reason: string) => ({
      route,
      account: null,
      verdict: "unknown" as Verdict,
      reasons: [reason],
      applicabilityEvidenceId: null,
      windows: [] as ShadowWindow[],
    });
    if (!Object.values(route).every(nonempty)) return unknown("invalid_route");
    const matches = applicability.filter((r) => routeKey(r) === key);
    if (matches.length !== 1) return unknown(matches.length ? "ambiguous_applicability" : "missing_applicability");
    const receipt = matches[0];
    if (!Number.isFinite(receipt.verifiedUntil) || receipt.verifiedUntil <= nowMs) {
      return unknown("applicability_not_current");
    }
    if (
      !nonempty(receipt.account) ||
      !nonempty(receipt.evidenceId) ||
      !receipt.required.length ||
      receipt.required.some(
        (w) => !nonempty(w.bucket) || !nonempty(w.kind) || !nonempty(w.period) || !Number.isFinite(w.resetsAt),
      ) ||
      new Set(receipt.required.map((w) => JSON.stringify([w.bucket, w.kind]))).size !== receipt.required.length
    )
      return unknown("invalid_applicability");

    const seat = projection.seats.find((s) => s.provider === route.provider && s.alias === route.alias);
    if (seat?.account !== receipt.account) return unknown("unverified_billing_account");

    const evidence = receipt.required.map(({ bucket, kind, period, resetsAt }) => {
      const rows = projection.accounts.filter(
        (a) => a.provider === route.provider && a.account === receipt.account && a.bucket === bucket,
      );
      if (rows.length !== 1 || !rows[0].aliases.includes(route.alias)) {
        return annotate(bucket, kind, "unknown", ["unverified_bucket"]);
      }
      const row = rows[0];
      const window = row.windows.find((w) => w.kind === kind && w.period === period && w.resetsAt === resetsAt);
      if (row.verdict === "stopped") {
        return annotate(
          bucket,
          kind,
          "stopped",
          ["active_stop"],
          window,
          row.activeStops.map((stop) => stop.id),
        );
      }
      if (!window) return annotate(bucket, kind, "unknown", ["unverified_window"]);
      if (row.reasons.includes("invalid_stop")) {
        return annotate(bucket, kind, worst([window.verdict, "unknown"]), [...window.reasons, "invalid_stop"], window);
      }
      return annotate(bucket, kind, window.verdict, window.reasons, window);
    });
    return {
      route,
      account: receipt.account,
      verdict: worst(evidence.map((x) => x.verdict)),
      reasons: [...new Set(evidence.flatMap((x) => x.reasons))].sort(),
      applicabilityEvidenceId: receipt.evidenceId,
      windows: evidence,
    };
  });
  const observed = actual ? decisions.find((d) => routeKey(d.route) === routeKey(actual)) : undefined;
  const comparison = !actual
    ? "not_observed"
    : !observed
      ? "not_candidate"
      : observed.account === null
        ? "account_unverified"
        : observed.account !== actual.account
          ? "account_mismatch"
          : observed.verdict;
  return {
    version: 1 as const,
    generation: projection.generation,
    evaluatedAt: projection.evaluatedAt,
    phase,
    decisions,
    wouldDefer: !decisions.some((d) => d.verdict === "eligible"),
    comparison,
  };
}
