// Flow analytics charts (#4830, step 3 of the #1281 split): the
// server-rendered charts for the #4829 flow-analytics JSON twin
// (/api/flow-analytics.json). Rendering only — this module computes
// nothing: every number it draws is read from the twin object the
// endpoint serves, so the charts can never disagree with the JSON
// (one computation path, #4829).
//
// Honesty rules, carried from the twin onto the page:
// - The computation window is stated on the section (first/last
//   cutoff, complete cutoffs of total), never left implicit.
// - Coverage gaps are named on the chart (cutoff + incomplete /
//   unavailable): intervals spanning a gap are excluded by the twin,
//   and the page says so instead of smoothing over them.
// - A complete week with zero completions renders as a zero bar with
//   a visible 0 — preserved by the twin, never dropped here.

import type { FlowAnalytics, FlowGroup, FlowWindow } from "./flow-analytics.ts";

function esc(s: unknown): string {
  return String(s ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

/** Days as the twin states them: integers plain, fractions to 2dp. */
function fmtDays(v: number): string {
  return Number.isInteger(v) ? String(v) : v.toFixed(2);
}

const TP_WIDTH = 560;
const TP_HEIGHT = 110;
const TP_BASELINE = 92;

/** The stated window + named coverage gaps for the whole twin. */
export function flowWindowHtml(window: FlowWindow): string {
  if (window.fromCutoff === null || window.toCutoff === null) {
    return `<div class="flow-window"><p class="muted">Flow analytics window: no item history stored yet — no cutoffs, no charts.</p></div>`;
  }
  const gaps = window.gaps.length
    ? `<div class="flag">Coverage gaps in this window: ${window.gaps
        .map(
          (g) =>
            `<span class="flow-gap" data-gap="${esc(g.cutoffAt)}">${esc(g.cutoffAt)} (${g.unavailable ? "unavailable" : "incomplete"})</span>`,
        )
        .join(", ")} — intervals spanning a gap are excluded from every number below; nothing is zero-filled or smoothed over.</div>`
    : "";
  return `<div class="flow-window" data-from="${esc(window.fromCutoff)}" data-to="${esc(window.toCutoff)}"><p class="muted">Computation window: ${esc(window.fromCutoff)} → ${esc(window.toCutoff)} · ${window.completeCutoffs} of ${window.cutoffCount} cutoffs complete. Source: <a href="/api/flow-analytics.json">/api/flow-analytics.json</a> — the charts below render that JSON twin and nothing else.</p>${gaps}</div>`;
}

/** Weekly throughput bar chart for one group, from the twin series. */
export function throughputChartHtml(group: FlowGroup): string {
  const weeks = group.throughput;
  if (weeks.length === 0) {
    return `<p class="muted">Throughput: no complete weeks in this window.</p>`;
  }
  const max = Math.max(1, ...weeks.map((w) => w.completed));
  const slot = TP_WIDTH / weeks.length;
  const barWidth = Math.max(6, slot * 0.62);
  const bars = weeks
    .map((w, i) => {
      const h = Math.round((w.completed / max) * (TP_BASELINE - 24));
      const x = i * slot + (slot - barWidth) / 2;
      const y = TP_BASELINE - h;
      return `<g class="flow-tp-week" data-cutoff="${esc(w.cutoffAt)}" data-completed="${w.completed}"><rect x="${x.toFixed(1)}" y="${y}" width="${barWidth.toFixed(1)}" height="${h}" class="flow-tp-bar"></rect><text x="${(i * slot + slot / 2).toFixed(1)}" y="${y - 4}" text-anchor="middle" class="flow-tp-value">${w.completed}</text><text x="${(i * slot + slot / 2).toFixed(1)}" y="${TP_BASELINE + 13}" text-anchor="middle" class="flow-tp-week-label">${esc(w.cutoffAt.slice(0, 10))}</text></g>`;
    })
    .join("");
  return `<div class="flow-chart"><h4>Weekly throughput (issues completed)</h4><svg viewBox="0 0 ${TP_WIDTH} ${TP_HEIGHT}" width="100%" role="img" aria-label="Weekly throughput">${bars}<line x1="0" y1="${TP_BASELINE}" x2="${TP_WIDTH}" y2="${TP_BASELINE}" class="flow-axis"></line></svg></div>`;
}

const TIS_WIDTH = 560;
const TIS_HEIGHT = 26;

/** Time-in-status stacked bar + exact-value legend for one group. */
export function timeInStatusHtml(group: FlowGroup): string {
  const total = group.timeInStatus.reduce((sum, s) => sum + s.itemDays, 0);
  if (total <= 0) {
    return `<p class="muted">Time in status: no complete-interval time recorded for this group in this window.</p>`;
  }
  let x = 0;
  const segs = group.timeInStatus
    .map((s, i) => {
      const w = (s.itemDays / total) * TIS_WIDTH;
      const out = `<rect class="flow-tis-seg flow-tis-${i % 6}" data-status="${esc(s.status)}" data-item-days="${s.itemDays}" x="${x.toFixed(1)}" y="0" width="${w.toFixed(1)}" height="${TIS_HEIGHT}"></rect>`;
      x += w;
      return out;
    })
    .join("");
  const legend = group.timeInStatus
    .map(
      (s) =>
        `<li data-status="${esc(s.status)}" data-item-days="${s.itemDays}">${esc(s.status)} — ${fmtDays(s.itemDays)} item-days</li>`,
    )
    .join("");
  return `<div class="flow-chart"><h4>Time in status (item-days)</h4><svg viewBox="0 0 ${TIS_WIDTH} ${TIS_HEIGHT}" width="100%" role="img" aria-label="Time in status">${segs}</svg><ul class="flow-tis-legend">${legend}</ul></div>`;
}

/** One group's charts: throughput + time-in-status. */
export function flowGroupHtml(group: FlowGroup): string {
  const name = group.groupRef === null ? "Unassigned" : group.groupRef;
  return `<div class="flow-group" data-group="${esc(group.groupRef ?? "")}"><h3>${esc(name)}</h3><p class="muted">${group.itemCount} item${group.itemCount === 1 ? "" : "s"} observed in this window.</p>${throughputChartHtml(group)}${timeInStatusHtml(group)}</div>`;
}

/** The full flow-analytics section for the Projects/Roadmap pages. */
export function flowChartsHtml(analytics: FlowAnalytics): string {
  const groups = analytics.groups.map(flowGroupHtml).join("");
  return `<section class="flow-analytics"><h2>Flow analytics</h2>${flowWindowHtml(analytics.window)}${groups || '<p class="muted">No groups observed in this window.</p>'}</section>`;
}
