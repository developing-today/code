// Rendering for the /quota "Analytics & projection" section
// (Drew's order, 2026-11-13): a cleaner key-value format over the
// model analytics-store.ts builds. Every number names its source in
// a footnote; inferred series (fleet subcommands from shell inputs)
// are labelled "inferred" at the row, never in fine print only.

import type { BurnBucket, PoolAnalytics, QuotaAnalytics } from "./analytics-store.ts";
import { fmtDuration, tokenRatios, type WowDelta } from "./quota-analytics.ts";

function esc(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

const whenFmt = new Intl.DateTimeFormat("en-US", {
  timeZone: "America/Chicago",
  weekday: "short",
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

function fmtWhen(ms: number | null): string {
  if (ms == null || !Number.isFinite(ms)) return "—";
  return whenFmt.format(new Date(ms));
}

function fmtIso(iso: string | null, now: number): string {
  if (!iso) return "—";
  const ms = Date.parse(iso);
  return Number.isFinite(ms) ? `${fmtWhen(ms)} (in ${fmtDuration(ms - now)})` : iso;
}

/** Compact counts: 789 · 45.6k · 1.23M · 5.78B. */
export function fmtN(n: number): string {
  const abs = Math.abs(n);
  if (abs >= 1e9) return `${(n / 1e9).toFixed(2)}B`;
  if (abs >= 1e6) return `${(n / 1e6).toFixed(2)}M`;
  if (abs >= 1e3) return `${(n / 1e3).toFixed(1)}k`;
  return `${Math.round(n)}`;
}

function fmtWow(w: WowDelta | null, unit: string): string {
  if (!w) return "";
  const sign = w.abs > 0 ? "+" : "";
  const pct = w.pct != null ? ` / ${sign}${w.pct.toFixed(0)}%` : "";
  return ` · vs last week ${sign}${fmtN(w.abs)}${unit}${pct}`;
}

function burnLine(label: string, b: BurnBucket | null): string {
  if (!b) return `<div>${esc(label)}: <span class="muted">no recorded burn</span></div>`;
  const r = tokenRatios(b);
  const cost =
    b.costUsd > 0
      ? ` · recorded cost $${b.costUsd.toFixed(2)} (on ${Math.round((b.costRows / Math.max(1, b.calls)) * 100)}% of calls)`
      : b.costRows > 0
        ? ` · no recorded cost (subscription legs record $0 in the ledger — not a price)`
        : "";
  const perDollar = b.costUsd > 0 ? ` · ${fmtN(r.total / b.costUsd)} tok/$ recorded` : "";
  return (
    `<div>${esc(label)}: <strong>${fmtN(r.total)} tok</strong> · ${fmtN(b.calls)} calls · ` +
    `in ${fmtN(b.input)} / out ${fmtN(b.output)} / cache-read ${fmtN(b.cacheRead)}` +
    `${r.inPerOut != null ? ` · in:out ${r.inPerOut.toFixed(1)}:1` : ""}` +
    `${r.cacheSharePct != null ? ` · cache ${r.cacheSharePct.toFixed(0)}% of input` : ""}${cost}${perDollar}</div>`
  );
}

function projectionText(p: PoolAnalytics["windows"][number], now: number): string {
  const pr = p.projection;
  switch (pr.state) {
    case "walled":
      return `<strong>walled now</strong>`;
    case "projected":
      return `projected wall <strong>${fmtWhen(pr.projectedWallAt)}</strong> (in ${fmtDuration(pr.durationUntilWallMs)}) — before the reset · burn ${pr.ratePtsPerHour?.toFixed(2)} pts/h over ${pr.samples} readings`;
    case "no-wall-before-reset":
      return pr.projectedWallAt != null
        ? `no wall before reset at the measured burn (${pr.ratePtsPerHour?.toFixed(2)} pts/h) — projected wall ${fmtWhen(pr.projectedWallAt)} lands after it`
        : `no wall before reset at the measured burn${pr.ratePtsPerHour != null ? ` (${pr.ratePtsPerHour.toFixed(2)} pts/h)` : ""}`;
    case "insufficient-history":
      return `<span class="muted">projection unavailable — ${esc(pr.reason ?? "not enough readings")}</span>`;
  }
}

function poolCard(p: PoolAnalytics, now: number, display: (n: string) => string): string {
  const windows = p.windows
    .map((w) => {
      const wowDur =
        w.wowDurationUntilWall && w.projection.durationUntilWallMs != null
          ? ` <span class="muted">· time-to-wall vs last week: ${w.wowDurationUntilWall.abs >= 0 ? "+" : ""}${w.wowDurationUntilWall.abs.toFixed(1)}h${w.wowDurationUntilWall.pct != null ? ` (${w.wowDurationUntilWall.abs >= 0 ? "+" : ""}${w.wowDurationUntilWall.pct.toFixed(0)}%)` : ""}</span>`
          : "";
      return (
        `<div><strong>${esc(w.name)}</strong>: <meter min="0" max="100" value="${Math.min(100, w.percent).toFixed(0)}"></meter> ${w.percent.toFixed(0)}% used` +
        ` · resets ${fmtIso(w.resetsAt, now)}` +
        `${w.durationUntilResetMs != null ? ` · <span class="muted">${fmtDuration(w.durationUntilResetMs)} until reset</span>` : ""}</div>` +
        `<div class="muted">${projectionText(w, now)}${wowDur}</div>`
      );
    })
    .join("");
  const unlock = p.unlock
    ? `<div>unlock: at the ${esc(p.bindingWindow ?? "")} reset (${fmtIso(p.unlock.bindingResetsAt, now)}) frees: ${p.unlock.freesAtReset.map(esc).join(", ") || "—"}` +
      `${p.unlock.stillWalledAfter.length ? ` · <strong>still walled after:</strong> ${p.unlock.stillWalledAfter.map((w) => `${esc(w.name)} (until ${fmtIso(w.resetsAt, now)})`).join(", ")}` : ` · nothing else stays walled`}</div>`
    : "";
  const legNote = p.legBasis
    ? `<div class="muted">burn legs: ${esc(p.legLabel ?? "")} · attribution by ${p.legBasis === "provider-id" ? "provider id" : p.legBasis === "shared-credential" ? "shared credential" : "credential id"} · source: route_attempt ledger</div>`
    : `<div class="muted">burn: no route leg maps to this pool yet</div>`;
  return (
    `<div class="card pool" id="pool-${esc(p.poolId)}"><div><strong>${esc(display(p.provider))}</strong><br>` +
    `<span class="muted">${esc(p.plan)}</span><br><span class="badge ${esc(p.state)}">${esc(p.state)}</span> ` +
    `<a href="#analytics-summary">↑ at-risk summary</a></div>` +
    `<div>${windows}${unlock}${burnLine("burn today", p.burnToday)}${burnLine("burn 7d", p.burn7d)}` +
    `${p.burnWowTokens ? `<div class="muted">7d tokens${fmtWow(p.burnWowTokens, "")}</div>` : ""}${legNote}</div></div>`
  );
}

function summaryStrip(a: QuotaAnalytics, display: (n: string) => string): string {
  const rows: { rank: number; html: string }[] = [];
  for (const p of a.pools) {
    for (const w of p.windows) {
      const pr = w.projection;
      const atRisk = w.percent >= 80 || pr.state === "walled" || pr.state === "projected";
      if (!atRisk) continue;
      const rank = pr.state === "walled" ? 0 : pr.state === "projected" ? 1 : 2;
      const verdict =
        pr.state === "walled"
          ? `<strong>walled</strong> · unlocks ${fmtIso(w.resetsAt, a.now)}`
          : pr.state === "projected"
            ? `projected wall <strong>${fmtWhen(pr.projectedWallAt)}</strong> (in ${fmtDuration(pr.durationUntilWallMs)}) · resets ${fmtIso(w.resetsAt, a.now)}`
            : `${w.percent.toFixed(0)}% · no measured wall before reset · resets ${fmtIso(w.resetsAt, a.now)}`;
      const still =
        p.unlock && p.bindingWindow === w.name && p.unlock.stillWalledAfter.length
          ? ` · after unlock: ${p.unlock.stillWalledAfter.map((s) => esc(s.name)).join(", ")} still walled`
          : "";
      rows.push({
        rank,
        html: `<div class="flag"><strong><a href="#pool-${esc(p.poolId)}">${esc(display(p.provider))} · ${esc(p.plan)} — ${esc(w.name)}</a></strong> — ${w.percent.toFixed(0)}% of limit used · ${verdict}${still}</div>`,
      });
    }
  }
  rows.sort((x, y) => x.rank - y.rank);
  if (!rows.length) return `<div class="card"><strong>At risk</strong> — no pool window is walled, projected to wall, or over 80%.</div>`;
  return rows.map((r) => r.html).join("");
}

function shareTable(title: string, groups: { label: string; calls: number; tokens: number; extra?: string }[]): string {
  const totalCalls = groups.reduce((s, g) => s + g.calls, 0);
  const totalTokens = groups.reduce((s, g) => s + g.tokens, 0);
  const rows = groups
    .map((g) => {
      const share = totalCalls > 0 ? (g.calls / totalCalls) * 100 : 0;
      return `<tr><th scope="row">${esc(g.label)}</th><td>${fmtN(g.calls)}</td><td><meter min="0" max="100" value="${share.toFixed(0)}"></meter> ${share.toFixed(0)}%</td><td>${fmtN(g.tokens)}</td><td>${totalTokens > 0 ? ((g.tokens / totalTokens) * 100).toFixed(0) : "0"}%</td>${g.extra ? `<td>${g.extra}</td>` : ""}</tr>`;
    })
    .join("");
  return `<h3>${esc(title)}</h3><div class="tblwrap"><table class="table table-zebra"><tr><th scope="col"></th><th scope="col">Calls</th><th scope="col">Call share</th><th scope="col">Tokens</th><th scope="col">Token share</th>${groups[0]?.extra ? "<th scope=\"col\">Detail</th>" : ""}</tr>${rows}</table></div>`;
}

function barStrip(values: { label: string; value: number; note?: string }[]): string {
  const max = Math.max(1, ...values.map((v) => v.value));
  return `<div>${values
    .map(
      (v) =>
        `<div>${esc(v.label)}: <meter min="0" max="${max}" value="${v.value}"></meter> ${fmtN(v.value)}${v.note ? ` <span class="muted">${esc(v.note)}</span>` : ""}</div>`,
    )
    .join("")}</div>`;
}

export function analyticsSectionHtml(a: QuotaAnalytics, display: (n: string) => string): string {
  if (!a.dbAvailable) {
    return (
      `<h2 id="analytics">Analytics &amp; projection</h2>` +
      `<div class="card"><strong>Analytics unavailable</strong> — the OpenCode route ledger could not be read on this host right now. ` +
      `Pool state above still comes from the quota gatherer. <span class="muted">Source: route_attempt ledger (opencode.db), read-only.</span></div>`
    );
  }
  const totals = a.totals;
  const modelGroups = a.byModel7d.slice(0, 12).map((g) => {
    const r = tokenRatios(g);
    return {
      label: g.label,
      calls: g.calls,
      tokens: r.total,
      extra: `in:out ${r.inPerOut != null ? `${r.inPerOut.toFixed(1)}:1` : "—"} · cache ${r.cacheSharePct != null ? `${r.cacheSharePct.toFixed(0)}%` : "—"}${g.costRows ? ` · $${g.costUsd.toFixed(2)} recorded` : ""}`,
    };
  });
  const effortGroups = a.byEffort7d.map((g) => ({ label: g.label, calls: g.calls, tokens: tokenRatios(g).total }));
  const legGroups = a.byLeg7d.slice(0, 12).map((g) => ({ label: display(g.label), calls: g.calls, tokens: tokenRatios(g).total }));
  const act = a.activity7d;
  const activityHtml = act
    ? `<div>sessions (7d): <strong>${fmtN(act.sessions)}</strong> · root ${fmtN(act.rootSessions)} · sub-sessions ${fmtN(act.subSessions)} · user turns ${fmtN(act.turns)}</div>` +
      `<div>tool calls (7d): <strong>${fmtN(act.toolCallsTotal)}</strong> · ${act.topTools.map((t) => `${esc(t.name)} ${fmtN(t.count)}`).join(" · ")}</div>` +
      `<div>skill use (7d): <strong>${fmtN(act.skillTotal)}</strong>${act.topSkills.length ? ` · ${act.topSkills.map((s) => `${esc(s.name)} ${fmtN(s.count)}`).join(" · ")}` : ""}</div>` +
      `<div>mcpx (7d): ${act.mcpx.length ? act.mcpx.map((m) => `${esc(m.name)} ${fmtN(m.count)}`).join(" · ") : "no direct mcpx tool calls"} · <span class="muted">execute gateway (code-tool calls): ${fmtN(act.executeGateway)}</span></div>` +
      `<div>fleet binary (7d, <strong>inferred</strong> from shell inputs — no invocation ledger exists): <strong>${fmtN(act.fleetInferredTotal)}</strong>${act.fleetInferred.length ? ` · ${act.fleetInferred.map((f) => `fleet ${esc(f.subcommand)} ${fmtN(f.count)}`).join(" · ")}` : ""}</div>`
    : `<div class="muted">activity scan (sessions, tools, skills, mcpx, fleet) is still computing — reload in about a minute.</div>`;
  const dowNames = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
  return (
    `<h2 id="analytics">Analytics &amp; projection</h2>` +
    `<p class="muted">Key values first: what is walled, what walls next and when, what unlocks at each reset and what stays walled. ` +
    `Burn from the route_attempt ledger (${a.attemptsFrom ? esc(a.attemptsFrom.slice(0, 10)) : "—"} → ${a.attemptsTo ? esc(a.attemptsTo.slice(0, 10)) : "—"}); ` +
    `projections from dated quota reads in the snapshot store. Sections: ` +
    `<a href="#analytics-summary">at-risk summary</a> · <a href="#analytics-pools">pool projections</a> · <a href="#analytics-breakdowns">breakdowns</a> · <a href="#analytics-patterns">time patterns</a> · <a href="#analytics-activity">activity</a> · ` +
    `data: <a href="/api/quota-analytics.json">/api/quota-analytics.json</a> · prices: <a href="/aa">model review</a> · rules: <a href="#model-choice">model choice</a></p>` +
    `<h3 id="analytics-summary">At-risk summary</h3>${summaryStrip(a, display)}` +
    `<div class="card"><div><strong>All-route burn</strong></div><div>` +
    burnLine("last hour", totals.lastHour) +
    burnLine("today", totals.today) +
    burnLine("last 7 days", totals.last7d) +
    `<div class="muted">7d tokens${fmtWow(a.totalsWowTokens, "")} · credential id recorded on ${a.credentialCoveragePct != null ? a.credentialCoveragePct.toFixed(0) : "—"}% of 7d calls (pool attribution coverage)</div>` +
    `</div></div>` +
    `<h3 id="analytics-pools">Pool projections</h3>${a.pools.map((p) => poolCard(p, a.now, display)).join("")}` +
    `<h3 id="analytics-breakdowns">Breakdowns — last 7 days</h3>` +
    shareTable("Model usage ratio", modelGroups) +
    shareTable("Effort (variant)", effortGroups) +
    shareTable("Provider legs", legGroups) +
    `<h3 id="analytics-patterns">Time patterns</h3>` +
    `<h4>Hour of day (calls, last 7d, local)</h4>${barStrip(a.hourly7d.map((h) => ({ label: `${String(h.hour).padStart(2, "0")}:00`, value: h.calls })))}` +
    `<h4>Day of week (calls, all recorded history)</h4>${barStrip(a.dowHistory.map((d) => ({ label: dowNames[d.dow] ?? `${d.dow}`, value: d.calls, note: `${fmtN(d.tokens)} tok` })))}` +
    `<h4>Daily (last 21 days)</h4>${barStrip(a.daily21d.map((d) => ({ label: d.day, value: d.calls, note: `${fmtN(d.tokens)} tok${d.holiday ? ` · ${d.holiday}` : ""}` })))}` +
    `<h3 id="analytics-activity">Activity</h3><div class="card"><div>${activityHtml}</div></div>`
  );
}
