// Spend beside progress for the Projects and Roadmap epic cards
// (#4824, slice 3 of the #1259 spend-rollups design,
// doc/spend-rollups-design-1259.md). The reader side of the #4823
// rollup store: this module turns stored rollup records into the
// per-epic card data the Projects pool cards and the Roadmap
// drill-down render.
//
// Honesty rules (the design's, enforced here and in tests):
// - The three unit families stay separate text-labelled measures —
//   tokens per pool, matched PAYG dollars, plan-pool share-of-window.
//   Nothing here computes a merged or total cost figure.
// - A scope with rollup records but no USD record renders its PAYG
//   figure as "unknown": the store emits no USD record when any
//   charge in the group was unknown, and unknown is never zero.
// - A window whose records are not fully covered renders labelled
//   partial, with the retained-history start named.
// - Forecast-to-complete comes only from a supplied forecast (the
//   #1256 contract's caller passes it in); absent renders "unknown",
//   never zero and never an invented number.
// - The unattributed bucket is a first-class card of its own, with
//   its reason-code breakdown behind a <details> disclosure (one
//   click) when the caller supplies the attribution projection's
//   reason lines.
import { selectRollups, type RollupCoverage, type RollupWindowKind, type SpendRollup } from "./spend-rollups.ts";

/** Forecast-to-complete for one scope, supplied by the caller from
 * the #1256 forecast contract. USD only — a forecast in any other
 * unit family is not rendered beside these measures. */
export interface SpendForecastToComplete {
  usd: number;
  confidence: string;
}

export interface SpendMeasure {
  /** Text label naming the unit family (and pool, where the unit is
   * per-pool). Columns are identified by this text, never by colour. */
  label: string;
  value: string;
}

export interface SpendCardData {
  scopeId: string;
  windowKind: RollupWindowKind;
  windowStart: string;
  windowEnd: string;
  coverage: RollupCoverage;
  /** Attribution version the selected records were computed under.
   * selectRollups picks one consistent version per window; printing
   * it on the view makes a restatement distinguishable from changed
   * consumption (design section 3, doc/spend-rollups-design-1259.md). */
  attributionVersion: string;
  /** Earliest retained source history, from the records. */
  coverageStart: string;
  measures: SpendMeasure[];
  forecast: SpendForecastToComplete | null;
}

export interface UnattributedReasonLine {
  reason: string;
  text: string;
}

export interface SpendCardsView {
  /** The window kind the cards describe; null when the store holds
   * no rollup records at all (no card renders — absence of data is
   * shown by absence, never by zeroed cards). */
  windowKind: RollupWindowKind | null;
  byScope: Map<string, SpendCardData>;
  unattributed: (SpendCardData & { reasons: UnattributedReasonLine[] }) | null;
}

export interface BuildSpendCardsOptions {
  /** Preferred window kind; falls back to the other kind when the
   * store holds no records of the preferred kind. Default "month". */
  windowKind?: RollupWindowKind;
  forecasts?: ReadonlyMap<string, SpendForecastToComplete>;
  unattributedReasons?: readonly UnattributedReasonLine[];
}

function fmtTokens(v: number): string {
  return Math.round(v).toLocaleString("en-US");
}

function fmtUsd(v: number): string {
  return `$${v.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
}

function fmtShare(v: number): string {
  return `${Number(v.toFixed(1))}%`;
}

function cardFor(
  scopeId: string,
  records: readonly SpendRollup[],
  forecasts: ReadonlyMap<string, SpendForecastToComplete> | undefined,
): SpendCardData {
  const first = records[0];
  const tokens = records.filter((r) => r.unit === "tokens").sort((a, b) => (a.pool ?? "").localeCompare(b.pool ?? ""));
  const shares = records
    .filter((r) => r.unit === "share_pct")
    .sort((a, b) => (a.pool ?? "").localeCompare(b.pool ?? ""));
  const usd = records.find((r) => r.unit === "usd");
  const measures: SpendMeasure[] = [
    ...tokens.map((r) => ({ label: `Tokens · pool ${r.pool}`, value: fmtTokens(r.value) })),
    // No USD record means some charge in the window was unknown —
    // render the design's "unknown", never a zero or a partial sum.
    { label: "PAYG dollars", value: usd ? fmtUsd(usd.value) : "unknown" },
    ...shares.map((r) => ({
      label: `Plan-pool share of window · pool ${r.pool}`,
      value: fmtShare(r.value),
    })),
  ];
  return {
    scopeId,
    windowKind: first.windowKind,
    windowStart: first.windowStart,
    windowEnd: first.windowEnd,
    coverage: records.some((r) => r.coverage === "partial") ? "partial" : "complete",
    attributionVersion: first.attributionVersion,
    coverageStart: records.map((r) => r.coverageStart).sort()[0],
    measures,
    forecast: forecasts?.get(scopeId) ?? null,
  };
}

/** Build the card view from stored rollup records (any attribution
 * versions — the store's consistent reader view is selected here).
 * Cards describe the LATEST window of the preferred kind present. */
export function buildSpendCards(records: readonly SpendRollup[], opts: BuildSpendCardsOptions = {}): SpendCardsView {
  const selected = selectRollups(records);
  const preferred = opts.windowKind ?? "month";
  const fallback: RollupWindowKind = preferred === "month" ? "week" : "month";
  const kind = [preferred, fallback].find((k) => selected.some((r) => r.windowKind === k));
  if (!kind) return { windowKind: null, byScope: new Map(), unattributed: null };
  const inKind = selected.filter((r) => r.windowKind === kind);
  const latestStart = inKind
    .map((r) => r.windowStart)
    .sort()
    .at(-1) as string;
  const inWindow = inKind.filter((r) => r.windowStart === latestStart);
  const byScopeRecords = new Map<string, SpendRollup[]>();
  for (const r of inWindow) {
    const list = byScopeRecords.get(r.scopeId) ?? [];
    list.push(r);
    byScopeRecords.set(r.scopeId, list);
  }
  const byScope = new Map<string, SpendCardData>();
  let unattributed: SpendCardsView["unattributed"] = null;
  for (const [scopeId, recs] of byScopeRecords) {
    const card = cardFor(scopeId, recs, opts.forecasts);
    if (recs[0].scopeKind === "unattributed") {
      unattributed = { ...card, reasons: [...(opts.unattributedReasons ?? [])] };
    } else {
      byScope.set(scopeId, card);
    }
  }
  return { windowKind: kind, byScope, unattributed };
}

// --- HTML (self-contained escaper: render.ts imports this module,
// --- so this module must not import render.ts back) -----------------

function esc(s: unknown): string {
  return String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

function monthLabel(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime())
    ? iso
    : d.toLocaleDateString("en-US", { month: "short", year: "numeric", timeZone: "UTC" });
}

function windowText(card: SpendCardData): string {
  const span = card.windowKind === "month" ? monthLabel(card.windowStart) : `week of ${card.windowStart.slice(0, 10)}`;
  const labelled = `${span} · attribution ${esc(card.attributionVersion)}`;
  return card.coverage === "partial"
    ? `${labelled} · partial window (source coverage starts ${esc(card.coverageStart.slice(0, 10))})`
    : labelled;
}

function measuresHtml(card: SpendCardData): string {
  const items = card.measures
    .map(
      (m) =>
        `<li><span class="spend-label">${esc(m.label)}</span>: <span class="spend-value">${esc(m.value)}</span></li>`,
    )
    .join("");
  const forecast = card.forecast
    ? `Forecast to complete: ${esc(fmtUsd(card.forecast.usd))} (confidence: ${esc(card.forecast.confidence)})`
    : "Forecast to complete: unknown";
  return `<ul class="spend-measures">${items}</ul><p class="muted spend-forecast">${forecast}</p>`;
}

/** One epic/project card's spend block: the three unit families as
 * text-labelled measures beside the card's progress rollup. */
export function spendCardHtml(card: SpendCardData): string {
  return `<div class="spend"><strong>Spend</strong> <span class="muted">(${esc(windowText(card))})</span>${measuresHtml(card)}</div>`;
}

/** The unattributed bucket as its own visible row, with the
 * reason-code breakdown one click away in a disclosure element. */
export function unattributedSpendHtml(card: SpendCardData & { reasons: UnattributedReasonLine[] }): string {
  const reasons = card.reasons.length
    ? `<ul>${card.reasons.map((r) => `<li>${esc(r.reason)}: ${esc(r.text)}</li>`).join("")}</ul>`
    : `<p class="muted">No reason breakdown recorded for this window yet — the attribution projection has not supplied one.</p>`;
  return `<div class="card spend-unattributed"><strong>Unattributed spend</strong> <span class="muted">(${esc(windowText(card))})</span>${measuresHtml(card)}<details class="spend-reasons"><summary>Reason-code breakdown</summary>${reasons}</details></div>`;
}
