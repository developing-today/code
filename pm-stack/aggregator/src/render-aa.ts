// render.ts split (#13051): AA cost-explorer view and its inline explorer script.
// render.ts re-exports every public name from here; callers keep importing "./render.ts".

import { aaCaiPoints, aaPoints, summarizeAa } from "./aa.ts";
import { ageMsText } from "./feed.ts";
import type { AaCaiRow, AaConfig, AaFeedSection, AaSection, AaTestRun, Provenance, ProvenanceDetail } from "./model.ts";

import { esc, fmtDate, freshnessChips, layout, pct } from "./render-core.ts";

// --- AA cost explorer page (ainventory import data via the fleet feed) -----

/**
 * The Cost Explorer's client side: tabs, the shared filter bar, the
 * three frontier charts, and both tables, all rendered from the
 * #xpl-data JSON the server embeds (aaPoints + catalog, computed in
 * src/aa.ts). Dependency-free inline script, like COMMENT_SCRIPT.
 * Written without template literals on purpose — it lives inside one.
 */
const AA_EXPLORER_SCRIPT = `<script type="module">
import { applyDominanceFilters, applyInequalityFilters, bandIndex, baselineRatios, chartClickSuppressed, computeRecommendations, costScoreEnvelope, costScoreOls, costScoreTail, effectiveChartView, logEdges, normName, panChartView, paretoSet, parseChartViewBox, pctRank, projectTask, resetChartView, scoreEdges, sortRows, unpricedScoredPoints, zoomChartView } from "/assets/aa-explorer-computations.js";
(function () {
  var dataEl = document.getElementById("xpl-data");
  if (!dataEl) return;
  var DATA = JSON.parse(dataEl.textContent);
  var AA_POINTS = DATA.points || [];
  var CAI_POINTS = DATA.caiPoints || [];
  var CATALOG = DATA.catalog || null;
  var POINTS = AA_POINTS;
  // Default benchmark vintage (2026-11-01, Drew): the payload carries
  // every vintage the AA import holds (v5 beside v4.3.2 today); ranking
  // across vintages compares scores from different methodologies, so
  // the first-load view pins the newest vintage present. "All
  // benchmarks" (bench "") stays one click away in the filter, and
  // saved setups / perspective switches keep their existing semantics.
  function benchVersionKey(b) {
    var m = /v(\d+(?:\.\d+)*)/.exec(b || "");
    return m ? m[1].split(".").map(function (n) { return parseInt(n, 10) || 0; }) : null;
  }
  function cmpVer(a, b) {
    var n = Math.max(a.length, b.length);
    for (var i = 0; i < n; i++) { var d = (a[i] || 0) - (b[i] || 0); if (d) return d; }
    return 0;
  }
  function latestBench(points) {
    var best = "", bestKey = null;
    (points || []).forEach(function (p) {
      var k = benchVersionKey(p.benchmark);
      if (!k) return;
      if (!bestKey || cmpVer(k, bestKey) > 0) { bestKey = k; best = p.benchmark; }
    });
    return best;
  }
  var PALETTE = ["#58a6ff","#3fb950","#e3b341","#f778ba","#bc8cff","#ffa657","#79c0ff","#56d364","#ff7b72","#a5d6ff","#7ee787","#f0883e","#d2a8ff","#e67e22","#2166ac","#b2182b","#1a9850","#386cb0","#7570b3","#66a61e","#e7298a","#bf812d","#0891b2","#ec4899","#2f9e6e","#d97706","#3b82f6","#a855f7","#94a3b8"];
  var famColor = {};
  var famList = [];
  // Colors are assigned over the union of both benchmark families so a
  // family keeps its color when the perspective switches.
  AA_POINTS.concat(CAI_POINTS).forEach(function (p) {
    if (!(p.family in famColor)) { famColor[p.family] = PALETTE[famList.length % PALETTE.length]; famList.push(p.family); }
  });
  var EORD = { "non-reasoning": 0, "low": 1, "medium": 2, "high": 3, "xhigh": 4, "max": 5, "reasoning": 6 };
  function eord(e) { var v = EORD[String(e || "").toLowerCase()]; return v == null ? 50 : v; }
  var ESHORT = { "medium": "med", "reasoning": "reas", "max (peak)": "max·pk", "max (off-peak, derived)": "max·offpk", "non-reasoning": "base" };
  function shortLabel(p) { var e = String(p.effort || ""); return p.family + " " + (ESHORT[e.toLowerCase()] || e); }
  var goCapByName = {};
  (CATALOG || []).forEach(function (e) { if (e.goCapUsd != null) goCapByName[normName(e.name)] = e.goCapUsd; });
  function goCapFor(p) { var v = goCapByName[normName(p.family)]; return v == null ? null : v; }
  function fmtInt(v) { return String(Math.round(v)).replace(/\B(?=(\d{3})+(?!\d))/g, ","); }
  function tasksAt(cost, budget) { return cost ? Math.floor(budget / cost) : null; }
  function esc(s) {
    return String(s == null ? "" : s).replace(/[&<>"']/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
    });
  }
  function fmt$(v) { return v == null ? "—" : "$" + (v < 0.01 ? v.toFixed(4) : v < 1 ? v.toFixed(3) : v.toFixed(2)); }
  function fmtT(v) {
    if (v == null) return "—";
    if (v < 60) return (Math.round(v * 10) / 10) + "s";
    return Math.floor(v / 60) + "m " + Math.round(v % 60) + "s";
  }
  function fmtN(v, d) { return v == null ? "—" : String(Math.round(v * Math.pow(10, d)) / Math.pow(10, d)); }
  function fmtCtx(v) {
    if (v == null) return "—";
    if (v >= 1000000) return (Math.round(v / 100000) / 10) + "M";
    if (v >= 1000) return Math.round(v / 1000) + "k";
    return String(v);
  }
  function cmpDesc(a, b) {
    if (a == null && b == null) return 0;
    if (a == null) return 1;
    if (b == null) return -1;
    return b - a;
  }
  function cmpAsc(a, b) {
    if (a == null && b == null) return 0;
    if (a == null) return 1;
    if (b == null) return -1;
    return a - b;
  }
  // --- benchmark perspective helpers -------------------------------------
  // The two benchmark families (AA Intelligence Index / Coding Agent
  // Index) are never converted into each other; the perspective only
  // swaps which point set is active and what its score is called.
  function scoreWord() { return S.persp === "cai" ? "CAI score" : "Intelligence"; }
  function scoreAxis() { return S.persp === "cai" ? "Coding Agent Index" : "Intelligence Index"; }
  function timeWord() { return S.persp === "cai" ? "agent wall time" : "time"; }
  var S = {
    tab: location.hash === "#models" ? "models" : "aa",
    persp: "aa",
    xp: "api",
    q: "",
    creator: "",
    fam: "",
    bench: latestBench(AA_POINTS),
    efforts: {},
    effortAny: false,
    providers: {},
    providerAny: false,
    off: {},
    elig: false,
    pooledOnly: false,
    measuredOnly: false,
    hideDisabled: false,
    sel: null,
    hidden: [],
    sizeBy: {},
    overlays: {},
    tasksMode: "per",
    baseline: "",
    pfCut: "50",
    bandCount: 3,
    bandScale: 1,
    rec: { allow: 5, speedPriority: 85, priceAllow: 500, slowAllow: 2, altWindow: 2, diversity: "lab", dominance: "lab", perBand: 3, maxCost: "", maxSec: "" },
    pj: { turns: 3, inputTokens: 20000, answerTokens: 2000, thinkingTokens: 4000, cacheReadPct: 50, writesPct: 0, overheadSec: 10, window: "peak" },
    ineq: { mode: "off", combine: "and", conds: [{ metric: "score", op: ">=", value: "" }, { metric: "cost", op: "<=", value: "" }, { metric: "time", op: "<=", value: "" }] },
    sortKey: "", sortDir: "desc", dom: { dominated: false, slower: false, dumber: false, pricier: false },
  };
  var XP_NOTES = {
    plan: "Plan perspective — plan quotas are sunk cost, so dollars are ignored: ranked on intelligence per second (speed + intelligence). Cost columns hidden.",
    api: "API perspective — ranked on intelligence per dollar and cost per task; the cost × score frontier is the value curve.",
    pertask: "Per-task perspective — ranked on total time and total cost to finish one task (the import's measured per-task figures, TTFT shown where timed).",
  };
  function basePoints() {
    return POINTS.filter(function (p) {
      if (S.q && (p.family + " " + (p.effort || "") + " " + p.creator).toLowerCase().indexOf(S.q) < 0) return false;
      if (S.creator && p.creator !== S.creator) return false;
      if (S.fam && p.family !== S.fam) return false;
      if (S.bench && p.benchmark !== S.bench) return false;
      if (S.effortAny && !S.efforts[p.effort || ""]) return false;
      if (S.elig && !p.eligible) return false;
      if (S.off[p.family]) return false;
      if (S.hidden.indexOf(p.key) >= 0) return false;
      return true;
    });
  }
  // --- inequality filters with Pareto-gap metrics (Site 1) ---------------
  // Gaps are measured against the observed frontier of the current scope
  // (all other filters applied): score gap = best score at or below this
  // cost minus this score; price premium = this cost over the cheapest
  // cost at or above this score; speed gap = score gap against time.
  function visPoints() {
    var base = basePoints();
    var filtered = applyInequalityFilters(base, S.ineq.mode, S.ineq.combine, S.ineq.conds);
    return applyDominanceFilters(filtered, S.dom);
  }
  function visCatalog() {
    if (!CATALOG) return [];
    return CATALOG.filter(function (e) {
      if (S.q && (e.id + " " + e.name + " " + e.providerName + " " + (e.plan || "")).toLowerCase().indexOf(S.q) < 0) return false;
      if (S.providerAny && !S.providers[e.provider]) return false;
      if (S.pooledOnly && !e.routes.length) return false;
      if (S.measuredOnly && e.aaScore == null) return false;
      if (S.hideDisabled && e.disabled) return false;
      return true;
    });
  }
  function sortPoints(rows) {
    var out = rows.slice();
    if (S.xp === "plan") {
      out.sort(function (a, b) { return cmpDesc(a.scorePerSecond, b.scorePerSecond) || cmpDesc(a.score, b.score); });
    } else if (S.xp === "pertask") {
      out.sort(function (a, b) { return cmpAsc(a.time, b.time) || cmpAsc(a.cost, b.cost); });
    } else {
      out.sort(function (a, b) {
        var ka = a.pointsPerDollar != null ? a.pointsPerDollar : a.scorePerDollar;
        var kb = b.pointsPerDollar != null ? b.pointsPerDollar : b.scorePerDollar;
        return cmpDesc(ka, kb) || cmpDesc(a.score, b.score);
      });
    }
    return out;
  }
  function effCost(e) {
    return e.aaCostPerTask != null && e.multiplier ? e.aaCostPerTask / e.multiplier : null;
  }
  function sortCatalog(rows) {
    var out = rows.slice();
    if (S.xp === "plan") {
      out.sort(function (a, b) {
        var ka = a.aaScore != null && a.aaTimePerTask ? a.aaScore / a.aaTimePerTask : null;
        var kb = b.aaScore != null && b.aaTimePerTask ? b.aaScore / b.aaTimePerTask : null;
        return cmpDesc(ka, kb) || cmpDesc(a.aaScore, b.aaScore) || (a.id < b.id ? -1 : 1);
      });
    } else if (S.xp === "pertask") {
      out.sort(function (a, b) { return cmpAsc(a.aaTimePerTask, b.aaTimePerTask) || cmpDesc(a.aaScore, b.aaScore); });
    } else {
      out.sort(function (a, b) { return cmpAsc(effCost(a), effCost(b)) || cmpDesc(a.aaScore, b.aaScore) || (a.id < b.id ? -1 : 1); });
    }
    return out;
  }
  // --- charts (hand-rolled SVG, the reference design's three scatters) ---
  var W = 680, H = 400, ML = 46, MR = 14, MT = 12, MB = 40, PW = W - ML - MR, PH = H - MT - MB;
  function logPos(v, lo, hi) { return (Math.log10(Math.max(v, lo)) - Math.log10(lo)) / (Math.log10(hi) - Math.log10(lo)); }
  function fmtTickX(v, kind) {
    if (kind === "cost") return "$" + (v < 1 ? v : Math.round(v));
    return v < 60 ? v + "s" : v / 60 + "m";
  }
  function drawChart(id, cfg) {
    var svg = document.getElementById(id);
    if (!svg) return;
    var all = visPoints();
    var rows = all.filter(function (p) { return cfg.x(p) != null && cfg.y(p) != null; });
    var note = svg.parentElement.querySelector("[data-note]");
    if (note) {
      var missing = all.filter(function (p) { return cfg.x(p) == null || cfg.y(p) == null; });
      var names = missing.slice(0, 4).map(shortLabel).join(", ");
      note.textContent = rows.length + " of " + all.length + " shown — the rest lack a measured " + cfg.need + " in the import (nothing is estimated)" +
        (missing.length ? ": " + names + (missing.length > 4 ? " (+" + (missing.length - 4) + " more)" : "") + "." : ".");
    }
    if (!rows.length) { svg.innerHTML = ""; return; }
    var xs = rows.map(cfg.x), ys = rows.map(cfg.y);
    var xlo, xhi, ylo, yhi;
    if (cfg.xlog) {
      xlo = Math.pow(10, Math.floor(Math.log10(Math.min.apply(null, xs)) * 2) / 2) / 1.4;
      xhi = Math.max.apply(null, xs) * 1.5;
    } else {
      xlo = Math.min.apply(null, xs); xhi = Math.max.apply(null, xs);
    }
    if (cfg.ylog) {
      ylo = Math.pow(10, Math.floor(Math.log10(Math.min.apply(null, ys)) * 2) / 2) / 1.4;
      yhi = Math.max.apply(null, ys) * 1.5;
    } else {
      ylo = Math.max(0, Math.floor(Math.min.apply(null, ys) - 4));
      yhi = Math.ceil(Math.max.apply(null, ys) + 3);
    }
    var X = function (v) { return ML + (cfg.xlog ? logPos(v, xlo, xhi) : (v - xlo) / (xhi - xlo)) * PW; };
    var Y = function (v) { return MT + (1 - (cfg.ylog ? logPos(v, ylo, yhi) : (v - ylo) / (yhi - ylo))) * PH; };
    var g = "";
    // gridlines + ticks
    var xticks = [];
    if (cfg.xlog) {
      for (var e = Math.ceil(Math.log10(xlo)); e <= Math.log10(xhi); e++) xticks.push(Math.pow(10, e));
      if (cfg.xkind === "time") xticks = [10, 60, 300, 900, 1800, 3600].filter(function (v) { return v >= xlo && v <= xhi; });
    }
    xticks.forEach(function (v) {
      g += '<line x1="' + X(v) + '" y1="' + MT + '" x2="' + X(v) + '" y2="' + (MT + PH) + '" stroke="#30363d" stroke-width="0.5" opacity="0.6"/>';
      g += '<text x="' + X(v) + '" y="' + (H - MB + 16) + '" font-size="9.5" fill="#8b949e" text-anchor="middle">' + fmtTickX(v, cfg.xkind) + "</text>";
    });
    var yticks = cfg.ylog ? [10, 60, 300, 900, 1800].filter(function (v) { return v >= ylo && v <= yhi; }) : [20, 30, 40, 50, 60].filter(function (v) { return v >= ylo && v <= yhi; });
    yticks.forEach(function (v) {
      g += '<line x1="' + ML + '" y1="' + Y(v) + '" x2="' + (ML + PW) + '" y2="' + Y(v) + '" stroke="#30363d" stroke-width="0.5" opacity="0.6"/>';
      g += '<text x="' + (ML - 5) + '" y="' + (Y(v) + 3) + '" font-size="9.5" fill="#8b949e" text-anchor="end">' + (cfg.ylog ? fmtTickX(v, "time") : v) + "</text>";
    });
    g += '<line x1="' + ML + '" y1="' + (MT + PH) + '" x2="' + (ML + PW) + '" y2="' + (MT + PH) + '" stroke="#30363d"/><line x1="' + ML + '" y1="' + MT + '" x2="' + ML + '" y2="' + (MT + PH) + '" stroke="#30363d"/>';
    // bracket dividers from the reference design (cost tiers, time splits, II lines)
    (cfg.dividers || []).forEach(function (dv) {
      if (dv.axis === "x") {
        if (dv.v < xlo || dv.v > xhi) return;
        var dx = X(dv.v);
        g += '<line x1="' + dx + '" y1="' + MT + '" x2="' + dx + '" y2="' + (MT + PH) + '" stroke="#8b949e" stroke-width="1" stroke-dasharray="4 4" opacity="0.55"/>';
        if (dv.label) g += '<text x="' + (dx + 3) + '" y="' + (MT + 11) + '" font-size="9" fill="#8b949e">' + dv.label + "</text>";
      } else {
        if (dv.v < ylo || dv.v > yhi) return;
        var dy = Y(dv.v);
        g += '<line x1="' + ML + '" y1="' + dy + '" x2="' + (ML + PW) + '" y2="' + dy + '" stroke="#8b949e" stroke-width="1" stroke-dasharray="4 4" opacity="0.55"/>';
        if (dv.label) g += '<text x="' + (ML + 3) + '" y="' + (dy - 3) + '" font-size="9" fill="#8b949e">' + dv.label + "</text>";
      }
    });
    g += '<text x="' + (ML + PW / 2) + '" y="' + (H - 4) + '" font-size="10.5" fill="#8b949e" text-anchor="middle">' + cfg.xlabel + "</text>";
    g += '<text x="12" y="' + (MT + PH / 2) + '" font-size="10.5" fill="#8b949e" text-anchor="middle" transform="rotate(-90 12 ' + (MT + PH / 2) + ')">' + cfg.ylabel.replace("Intelligence Index", scoreAxis()) + "</text>";
    // effort ladder lines per family (kept readable: only when few points show)
    if (rows.length <= 60) {
      famList.forEach(function (fam) {
        if (S.off[fam]) return;
        var pts = rows.filter(function (p) { return p.family === fam; }).sort(function (a, b) { return eord(a.effort) - eord(b.effort); });
        if (pts.length < 2) return;
        var isDS = fam.indexOf("DeepSeek") === 0;
        g += '<polyline points="' + pts.map(function (p) { return X(cfg.x(p)).toFixed(1) + "," + Y(cfg.y(p)).toFixed(1); }).join(" ") + '" fill="none" stroke="' + famColor[fam] + '" stroke-width="' + (isDS ? 1.6 : 1) + '" opacity="' + (isDS ? 0.8 : 0.4) + '"' + (isDS ? ' stroke-dasharray="2.5 3"' : "") + "/>";
      });
    }
    // frontier curve for this chart's axis pair
    var fr = rows.filter(function (p) { return p[cfg.flag]; }).sort(function (a, b) { return cfg.x(a) - cfg.x(b); });
    if (fr.length > 1) {
      g += '<polyline points="' + fr.map(function (p) { return X(cfg.x(p)).toFixed(1) + "," + Y(cfg.y(p)).toFixed(1); }).join(" ") + '" fill="none" stroke="#e6edf3" stroke-width="1.4" opacity="0.65" stroke-dasharray="5 3"/>';
    }
    // comparison-line overlays (#1400): Cost x Score only, toggleable,
    // off by default. All three read the same visible rows the dots use
    // and draw in data coordinates through this chart's own X/Y scales.
    if (id === "xpl-c2") {
      var ovRows = rows.map(function (p) { return { cost: cfg.x(p), score: cfg.y(p) }; });
      if (S.overlays.trend) {
        var fit = costScoreOls(ovRows);
        if (fit) {
          var tLo = Math.max(xlo, fit.costMin), tHi = Math.min(xhi, fit.costMax);
          var yAt = function (c) {
            var y = fit.intercept + fit.slope * Math.log10(Math.max(c, tLo));
            return Math.min(Math.max(y, ylo), yhi);
          };
          g += '<polyline class="xpl-overlay" points="' + X(tLo).toFixed(1) + "," + Y(yAt(tLo)).toFixed(1) + " " + X(tHi).toFixed(1) + "," + Y(yAt(tHi)).toFixed(1) + '" fill="none" stroke="#e3b341" stroke-width="1.6" opacity="0.85"><title>OLS cost–score trend (score on log10 cost, n=' + fit.n + ")</title></polyline>";
        }
      }
      if (S.overlays.envelope) {
        var env = costScoreEnvelope(ovRows);
        if (env.length > 1) {
          g += '<polyline class="xpl-overlay" points="' + env.map(function (pt) { return X(pt.cost).toFixed(1) + "," + Y(pt.score).toFixed(1); }).join(" ") + '" fill="none" stroke="#3fb950" stroke-width="1.6" opacity="0.85" stroke-dasharray="7 3"><title>Smoothed upper-envelope frontier (cost/score Pareto set)</title></polyline>';
        }
      }
      if (S.overlays.tail) {
        var tail = costScoreTail(ovRows, 8);
        if (tail.length > 1) {
          g += '<polyline class="xpl-overlay" points="' + tail.map(function (pt) { return X(pt.cost).toFixed(1) + "," + Y(pt.score).toFixed(1); }).join(" ") + '" fill="none" stroke="#79c0ff" stroke-width="1.6" opacity="0.85" stroke-dasharray="2 3"><title>Smoothed upper-tail expectation (mean of the top half per log-cost bin)</title></polyline>';
        }
      }
    }
    // dot sizing by the chart's third metric (reference design's toggle)
    var sizeVals = null, sizeLo = 0, sizeHi = 1;
    if (S.sizeBy[id] && cfg.sizeMetric) {
      sizeVals = rows.map(cfg.sizeMetric).filter(function (v) { return v != null; });
      if (sizeVals.length) {
        var tr = cfg.sizeLog ? sizeVals.map(function (v) { return Math.log10(v); }) : sizeVals;
        sizeLo = Math.min.apply(null, tr); sizeHi = Math.max.apply(null, tr);
      }
    }
    function radius(p, onF) {
      if (!sizeVals || !sizeVals.length || !cfg.sizeMetric) return onF ? 5.5 : 4.4;
      var v = cfg.sizeMetric(p);
      if (v == null) return 3;
      var t = cfg.sizeLog ? Math.log10(v) : v;
      var f = sizeHi > sizeLo ? Math.min(1, Math.max(0, (t - sizeLo) / (sizeHi - sizeLo))) : 0.5;
      return 1.6 + Math.pow(f, 1.5) * 8.4;
    }
    // 2026-11-01 (Drew, code#3335): every dot carries its family color;
    // frontier membership is emphasis (white ring, full opacity), not
    // the only color — on real data a chart's frontier can be a single
    // dominant point, which used to grey out the whole chart.
    rows.forEach(function (p) {
      var cx = X(cfg.x(p)), cy = Y(cfg.y(p));
      var onF = !!p[cfg.flag];
      var col = famColor[p.family];
      var r = radius(p, onF);
      var hollow = p.benchmark && p.benchmark.indexOf("AA") !== 0;
      var tip = p.family + " " + (p.effort || "") + " — " + scoreWord() + " " + fmtN(p.score, 1) + " · " + fmt$(p.cost) + "/task · " + fmtT(p.time) + "/task" + (p.tps != null ? " · " + p.tps + " tok/s" : "") + (hollow ? " · " + p.benchmark + " basis" : "");
      g += '<circle class="xpt" data-key="' + esc(p.key) + '" cx="' + cx.toFixed(1) + '" cy="' + cy.toFixed(1) + '" r="' + r.toFixed(1) + '" fill="' + col + '" fill-opacity="' + (hollow ? 0.25 : onF ? 0.95 : 0.45) + '" stroke="' + (S.sel === p.key || onF ? "#fff" : col) + '" stroke-width="' + (S.sel === p.key ? 2 : onF ? 1.8 : 1) + '"' + (hollow ? ' stroke-dasharray="2.5 1.8"' : "") + "><title>" + esc(tip) + "</title></circle>";
      if (onF || S.sel === p.key) {
        var anchorEnd = cx > ML + PW * 0.78;
        g += '<text data-key="' + esc(p.key) + '" x="' + (anchorEnd ? cx - r - 3 : cx + r + 3) + '" y="' + (cy + 3) + '" font-size="8.6" fill="' + (onF ? "#c9d1d9" : "#8b949e") + '" text-anchor="' + (anchorEnd ? "end" : "start") + '" style="cursor:pointer">' + esc(shortLabel(p)) + "</text>";
      }
    });
    svg.innerHTML = g;
    svg.querySelectorAll("[data-key]").forEach(function (el) {
      el.addEventListener("click", function () { selectPoint(el.getAttribute("data-key")); });
    });
  }
  var COST_DIVIDERS = [0.15, 0.75, 2.5].map(function (v, i) {
    return { axis: "x", v: v, label: ["Mid", "Premium", "Flagship"][i] };
  });
  var TIME_DIVIDERS_X = [60, 300, 900].map(function (v) { return { axis: "x", v: v, label: "" }; });
  var TIME_DIVIDERS_Y = [60, 300, 900].map(function (v) { return { axis: "y", v: v, label: "" }; });
  var CHARTS = [
    { id: "xpl-c1", x: function (p) { return p.time; }, y: function (p) { return p.score; }, xlog: true, ylog: false, xkind: "time", xlabel: "Seconds to finish one task (log)", ylabel: "Intelligence Index", flag: "frontierTime", need: "per-task time", sizeMetric: function (p) { return p.cost; }, sizeLog: true, dividers: TIME_DIVIDERS_X },
    { id: "xpl-c2", x: function (p) { return p.cost; }, y: function (p) { return p.score; }, xlog: true, ylog: false, xkind: "cost", xlabel: "Cost per task, USD (log)", ylabel: "Intelligence Index", flag: "frontierCost", need: "cost", sizeMetric: function (p) { return p.time; }, sizeLog: true, dividers: COST_DIVIDERS.concat([{ axis: "y", v: 40, label: "II 40" }, { axis: "y", v: 50, label: "II 50" }]) },
    { id: "xpl-c3", x: function (p) { return p.cost; }, y: function (p) { return p.time; }, xlog: true, ylog: true, xkind: "cost", xlabel: "Cost per task, USD (log)", ylabel: "Seconds per task (log)", flag: "frontierCostTime", need: "cost + per-task time", sizeMetric: function (p) { return p.score; }, sizeLog: false, dividers: COST_DIVIDERS.concat(TIME_DIVIDERS_Y) },
  ];
  var CHART_ORDER = { plan: ["xpl-c1", "xpl-c2", "xpl-c3"], api: ["xpl-c2", "xpl-c1", "xpl-c3"], pertask: ["xpl-c3", "xpl-c2", "xpl-c1"] };
  // --- chart expand + zoom/pan (#3298) -------------------------------
  // Every chart on this page is an SVG with a viewBox, so one uniform
  // layer covers all of them: the six explorer charts (tool buttons
  // are server-rendered next to each chart) and the band scatters
  // built by renderRecs / renderBandComparisons (tools injected
  // here). Zoom/pan manipulates only the viewBox attribute; draws
  // rewrite innerHTML (and, for the scores chart, the viewBox
  // height), so this layer tracks the last viewBox it applied — a
  // different attribute at enhance time means a draw reset the
  // chart and any zoom state for it is dropped. State is per chart;
  // Reset restores the drawn viewBox. Pan only acts while zoomed.
  var chartViews = {};
  function chartKeyFor(svg) {
    if (svg.id) return svg.id;
    if (!svg.getAttribute("data-zkey")) svg.setAttribute("data-zkey", "zx" + Math.random().toString(36).slice(2, 9));
    return svg.getAttribute("data-zkey");
  }
  function parseViewBox(s) { return parseChartViewBox(s); }
  function chartState(svg) {
    var key = chartKeyFor(svg);
    var attr = svg.getAttribute("viewBox") || "";
    var st = chartViews[key];
    if (!st || st.applied !== attr) {
      var base = parseViewBox(attr);
      if (!base) return null;
      st = chartViews[key] = { base: base, cur: null, applied: attr };
    }
    return st;
  }
  function applyChartView(svg, st) {
    var v = effectiveChartView(st);
    var s = v.join(" ");
    svg.setAttribute("viewBox", s);
    st.applied = s;
  }
  // The viewBox math itself lives in aa-explorer-computations.js
  // (#9581, same pattern as #7562): these wrappers only adapt the
  // module functions to an svg's chart state — no inline copies, so
  // the Bun tests exercise exactly the code the browser runs.
  function zoomChart(svg, factor, fx, fy) {
    var st = chartState(svg);
    if (!st) return;
    zoomChartView(st, factor, fx, fy);
    applyChartView(svg, st);
  }
  function panChart(svg, dx, dy) {
    var st = chartState(svg);
    if (!st || !st.cur) return;
    var r = svg.getBoundingClientRect();
    if (!panChartView(st, dx, dy, r.width, r.height)) return;
    applyChartView(svg, st);
  }
  function resetChart(svg) {
    var st = chartState(svg);
    if (!st) return;
    resetChartView(st);
    applyChartView(svg, st);
  }
  function chartHost(svg) { return svg.closest(".chart") || svg.parentElement; }
  function setChartExpanded(svg, on) {
    var host = chartHost(svg);
    if (!host) return;
    host.classList.toggle("chart-expanded", on);
    var key = chartKeyFor(svg);
    document.querySelectorAll('[data-zact="expand"]').forEach(function (b) {
      if (b.getAttribute("data-zfor") === key) {
        b.textContent = on ? "✕ Close" : "⤢ Expand";
        b.setAttribute("aria-expanded", on ? "true" : "false");
      }
    });
  }
  function chartToolsHtml(key) {
    return '<span class="chart-tools" data-tools="' + key + '">' +
      '<button type="button" data-zact="expand" data-zfor="' + key + '">⤢ Expand</button>' +
      '<button type="button" data-zact="in" data-zfor="' + key + '" aria-label="Zoom in">+</button>' +
      '<button type="button" data-zact="out" data-zfor="' + key + '" aria-label="Zoom out">−</button>' +
      '<button type="button" data-zact="reset" data-zfor="' + key + '" aria-label="Reset view">⟲</button></span>';
  }
  function findChartSvg(key) {
    var byId = document.getElementById(key);
    if (byId) return byId;
    return document.querySelector('svg[data-zkey="' + key + '"]');
  }
  function enhanceChart(svg) {
    var st = chartState(svg);
    if (!st) return;
    if (st.cur) applyChartView(svg, st);
    if (svg.getAttribute("data-zbound")) return;
    svg.setAttribute("data-zbound", "1");
    svg.classList.add("chart-zoomable");
    // Band scatters are bare children of the shared #xpl-recs /
    // #xpl-bandgroups containers (Codex P2, PR #4154): give each a
    // dedicated wrapper so Expand isolates this chart instead of
    // overlaying the whole section.
    var zparent = svg.parentElement;
    if (zparent && (zparent.id === "xpl-recs" || zparent.id === "xpl-bandgroups")) {
      var mini = document.createElement("div");
      mini.className = "chart-mini";
      zparent.insertBefore(mini, svg);
      mini.appendChild(svg);
    }
    var key = chartKeyFor(svg);
    if (!document.querySelector('[data-tools="' + key + '"]')) {
      var host = chartHost(svg);
      if (host) {
        var tmp = document.createElement("span");
        tmp.innerHTML = chartToolsHtml(key);
        host.insertBefore(tmp.firstChild, svg);
      }
    }
    var drag = null;
    svg.addEventListener("pointerdown", function (e) {
      if (e.button !== 0) return;
      // No pointer capture yet (Codex P1, PR #4154): capturing here
      // would retarget an ordinary click away from the point/bar
      // selection handlers on the svg's descendants. Capture only
      // once pointermove detects an actual pan gesture.
      drag = { lx: e.clientX, ly: e.clientY, sx: e.clientX, sy: e.clientY, moved: false, id: e.pointerId };
    });
    svg.addEventListener("pointermove", function (e) {
      if (!drag) return;
      var dx = e.clientX - drag.lx, dy = e.clientY - drag.ly;
      drag.lx = e.clientX; drag.ly = e.clientY;
      if (!drag.moved && Math.abs(e.clientX - drag.sx) + Math.abs(e.clientY - drag.sy) > 4) {
        drag.moved = true;
        try { svg.setPointerCapture(drag.id); } catch (err) {}
      }
      if (drag.moved) panChart(svg, dx, dy);
    });
    svg.addEventListener("pointerup", function () {
      if (drag && drag.moved) svg.setAttribute("data-zsuppress", String(Date.now()));
      drag = null;
    });
    svg.addEventListener("click", function (e) {
      if (chartClickSuppressed(svg.getAttribute("data-zsuppress"), Date.now())) { e.stopPropagation(); e.preventDefault(); svg.removeAttribute("data-zsuppress"); }
    }, true);
    svg.addEventListener("wheel", function (e) {
      e.preventDefault();
      var r = svg.getBoundingClientRect();
      var fx = r.width ? (e.clientX - r.left) / r.width : 0.5;
      var fy = r.height ? (e.clientY - r.top) / r.height : 0.5;
      zoomChart(svg, e.deltaY < 0 ? 0.85 : 1.18, fx, fy);
    }, { passive: false });
  }
  function enhanceCharts() {
    document.querySelectorAll("#xpl-charts svg, #xpl-recs svg, #xpl-bandgroups svg").forEach(function (svg) { enhanceChart(svg); });
  }
  document.addEventListener("click", function (e) {
    var b = e.target.closest("[data-zact]");
    if (!b) return;
    var svg = findChartSvg(b.getAttribute("data-zfor"));
    if (!svg) return;
    var act = b.getAttribute("data-zact");
    if (act === "expand") {
      var host = chartHost(svg);
      setChartExpanded(svg, !(host && host.classList.contains("chart-expanded")));
    } else if (act === "in") zoomChart(svg, 0.8, null, null);
    else if (act === "out") zoomChart(svg, 1.25, null, null);
    else if (act === "reset") resetChart(svg);
  });
  document.addEventListener("keydown", function (e) {
    if (e.key !== "Escape") return;
    document.querySelectorAll(".chart-expanded").forEach(function (h) { h.classList.remove("chart-expanded"); });
    document.querySelectorAll('[data-zact="expand"]').forEach(function (b) { b.textContent = "⤢ Expand"; b.setAttribute("aria-expanded", "false"); });
  });
  function drawCharts() {
    var wrap = document.getElementById("xpl-charts");
    var order = CHART_ORDER[S.xp];
    wrap.querySelectorAll(".chart").forEach(function (c) {
      c.classList.remove("primary");
      var idx = order.indexOf(c.getAttribute("data-chart"));
      c.style.order = idx >= 0 ? String(idx + 1) : "9";
    });
    var first = wrap.querySelector('[data-chart="' + order[0] + '"]');
    if (first) first.classList.add("primary");
    CHARTS.forEach(function (c) { drawChart(c.id, c); });
    drawTasks();
    drawMultiples();
    drawScores();
  }
  function downloadFile(name, text, type) {
    var blob = new Blob([text], { type: type || "text/plain" });
    var url = URL.createObjectURL(blob);
    var a = document.createElement("a");
    a.download = name;
    a.href = url;
    a.click();
    setTimeout(function () { URL.revokeObjectURL(url); }, 500);
  }
  function downloadPng(svgId) {
    var svg = document.getElementById(svgId);
    if (!svg) return;
    var vb = (svg.getAttribute("viewBox") || "0 0 680 400").split(" ");
    var w = Math.round(Number(vb[2]) * 2), h = Math.round(Number(vb[3]) * 2);
    var clone = svg.cloneNode(true);
    clone.setAttribute("xmlns", "http://www.w3.org/2000/svg");
    clone.setAttribute("width", String(w));
    clone.setAttribute("height", String(h));
    var blob = new Blob([new XMLSerializer().serializeToString(clone)], { type: "image/svg+xml;charset=utf-8" });
    var url = URL.createObjectURL(blob);
    var img = new Image();
    img.onload = function () {
      var cv = document.createElement("canvas");
      cv.width = w; cv.height = h;
      var ctx = cv.getContext("2d");
      ctx.fillStyle = "#0d1117";
      ctx.fillRect(0, 0, w, h);
      ctx.drawImage(img, 0, 0, w, h);
      URL.revokeObjectURL(url);
      var a = document.createElement("a");
      a.download = "cost-explorer-" + svgId + ".png";
      a.href = cv.toDataURL("image/png");
      a.click();
    };
    img.src = url;
  }
  // --- chart 4: tasks completed vs duration (scatter-widget parity) ---
  var BUDGET_LADDER = [0.25, 1, 2, 5, 10, 20, 50, 100, 120, 200];
  function nominalBudget(cost) {
    var target = 30 * cost, best = BUDGET_LADDER[0], bd = Infinity;
    BUDGET_LADDER.forEach(function (b) {
      var d = Math.abs(Math.log10(b / target));
      if (d < bd) { bd = d; best = b; }
    });
    return best;
  }
  function drawTasks() {
    var svg = document.getElementById("xpl-c4");
    if (!svg) return;
    var all = visPoints();
    var rows = all.filter(function (p) { return p.cost != null && p.time != null; });
    var note = svg.parentElement.querySelector("[data-note]");
    if (note) {
      var missing = all.filter(function (p) { return p.cost == null || p.time == null; });
      note.textContent = rows.length + " of " + all.length + " shown — the rest lack a measured cost or per-task time: " +
        missing.slice(0, 4).map(shortLabel).join(", ") + (missing.length > 4 ? " (+" + (missing.length - 4) + " more)" : "") +
        ". Circles = a nominal budget ($0.25–$200, picked per model so its task count lands in range). ◆ = the model's published OpenCode Go monthly cap spent on that model alone (docs reading, 2026-10-20 — the fleet's workspaces appear to meter one pooled bar instead). Hollow = non-AA benchmark basis.";
    }
    document.querySelectorAll("[data-tmode]").forEach(function (b) {
      b.classList.toggle("active", b.getAttribute("data-tmode") === S.tasksMode);
    });
    if (!rows.length) { svg.innerHTML = ""; return; }
    var per = S.tasksMode === "per";
    var pts = rows.map(function (p) {
      var budget = nominalBudget(p.cost);
      var tasks = Math.max(1, Math.floor(budget / p.cost));
      var cap = goCapFor(p);
      return { p: p, budget: budget, tasks: tasks, y: per ? p.time : tasks * p.time, goTasks: cap ? Math.floor(cap / p.cost) : null, goY: cap ? (per ? p.time : Math.floor(cap / p.cost) * p.time) : null };
    });
    var xs = pts.map(function (t) { return t.tasks; }).concat(pts.filter(function (t) { return t.goTasks; }).map(function (t) { return t.goTasks; }));
    var ys = pts.map(function (t) { return t.y; }).concat(pts.filter(function (t) { return t.goY != null; }).map(function (t) { return t.goY; }));
    var xlo = Math.pow(10, Math.floor(Math.log10(Math.min.apply(null, xs)) * 2) / 2) / 1.4;
    var xhi = Math.max.apply(null, xs) * 1.6;
    var ylo = Math.pow(10, Math.floor(Math.log10(Math.min.apply(null, ys)) * 2) / 2) / 1.4;
    var yhi = Math.max.apply(null, ys) * 1.5;
    var X = function (v) { return ML + logPos(v, xlo, xhi) * PW; };
    var Y = function (v) { return MT + (1 - logPos(v, ylo, yhi)) * PH; };
    var g = "";
    [1, 3, 10, 30, 100, 300, 1000, 3000].filter(function (v) { return v >= xlo && v <= xhi; }).forEach(function (v) {
      g += '<line x1="' + X(v) + '" y1="' + MT + '" x2="' + X(v) + '" y2="' + (MT + PH) + '" stroke="#30363d" stroke-width="0.5" opacity="0.6"/><text x="' + X(v) + '" y="' + (H - MB + 16) + '" font-size="9.5" fill="#8b949e" text-anchor="middle">' + v + "</text>";
    });
    [10, 30, 100, 300, 1000, 3000, 10000, 43200, 140000].filter(function (v) { return v >= ylo && v <= yhi; }).forEach(function (v) {
      g += '<line x1="' + ML + '" y1="' + Y(v) + '" x2="' + (ML + PW) + '" y2="' + Y(v) + '" stroke="#30363d" stroke-width="0.5" opacity="0.6"/><text x="' + (ML - 5) + '" y="' + (Y(v) + 3) + '" font-size="9.5" fill="#8b949e" text-anchor="end">' + fmtT(v) + "</text>";
    });
    g += '<line x1="' + ML + '" y1="' + (MT + PH) + '" x2="' + (ML + PW) + '" y2="' + (MT + PH) + '" stroke="#30363d"/><line x1="' + ML + '" y1="' + MT + '" x2="' + ML + '" y2="' + (MT + PH) + '" stroke="#30363d"/>';
    g += '<text x="' + (ML + PW / 2) + '" y="' + (H - 4) + '" font-size="10.5" fill="#8b949e" text-anchor="middle">tasks completed (log scale)</text>';
    g += '<text x="12" y="' + (MT + PH / 2) + '" font-size="10.5" fill="#8b949e" text-anchor="middle" transform="rotate(-90 12 ' + (MT + PH / 2) + ')">' + (per ? "duration per task" : "total time, all tasks back-to-back") + "</text>";
    pts.forEach(function (t) {
      var p = t.p, cx = X(t.tasks), cy = Y(t.y), col = famColor[p.family];
      var hollow = p.benchmark && p.benchmark.indexOf("AA") !== 0;
      var tip = shortLabel(p) + " — $" + t.budget + " nominal → " + t.tasks + " tasks · " + fmtT(p.time) + "/task · " + fmt$(p.cost) + "/task · " + scoreWord() + " " + fmtN(p.score, 1) + (t.goTasks ? " · published Go cap buys " + fmtInt(t.goTasks) + " tasks" : "");
      g += '<circle class="xpt" data-key="' + esc(p.key) + '" cx="' + cx.toFixed(1) + '" cy="' + cy.toFixed(1) + '" r="5.5" fill="' + col + '" fill-opacity="' + (hollow ? 0.25 : 0.9) + '" stroke="' + (S.sel === p.key ? "#fff" : col) + '" stroke-width="' + (S.sel === p.key ? 2 : 1.5) + '"' + (hollow ? ' stroke-dasharray="2.5 1.8"' : "") + "><title>" + esc(tip) + "</title></circle>";
      if (t.goTasks) {
        var gx = X(t.goTasks), gy = Y(t.goY);
        g += '<rect data-key="' + esc(p.key) + '" x="' + (gx - 5).toFixed(1) + '" y="' + (gy - 5).toFixed(1) + '" width="10" height="10" transform="rotate(45 ' + gx.toFixed(1) + " " + gy.toFixed(1) + ')" fill="' + col + '" fill-opacity="0.95" stroke="#0d1117" stroke-width="1.2" style="cursor:pointer"><title>' + esc(tip) + "</title></rect>";
      }
      if (p.frontierCostTime || S.sel === p.key) {
        var anchorEnd = cx > ML + PW * 0.78;
        g += '<text data-key="' + esc(p.key) + '" x="' + (anchorEnd ? cx - 9 : cx + 9) + '" y="' + (cy + 3) + '" font-size="8.6" fill="#c9d1d9" text-anchor="' + (anchorEnd ? "end" : "start") + '" style="cursor:pointer">' + esc(shortLabel(p)) + "</text>";
      }
    });
    svg.innerHTML = g;
    svg.querySelectorAll("[data-key]").forEach(function (el) {
      el.addEventListener("click", function () { selectPoint(el.getAttribute("data-key")); });
    });
  }
  // --- chart 5: multiples of a baseline (multiples-widget parity) ---
  function baselinePoint() {
    var pool = POINTS.filter(function (p) { return p.cost != null && p.time != null && p.score != null && S.hidden.indexOf(p.key) < 0; });
    if (!pool.length) return null;
    var chosen = pool.find(function (p) { return p.key === S.baseline; });
    if (chosen) return chosen;
    var lunaLow = pool.find(function (p) { return p.family.indexOf("Luna") >= 0 && String(p.effort).toLowerCase() === "low"; });
    if (lunaLow) return lunaLow;
    return pool.slice().sort(function (a, b) { return a.cost - b.cost; })[0];
  }
  function buildBaselineOptions() {
    var sel = document.getElementById("xf-baseline");
    if (!sel) return;
    var pool = POINTS.filter(function (p) { return p.cost != null && p.time != null && p.score != null; })
      .sort(function (a, b) { return a.family < b.family ? -1 : a.family > b.family ? 1 : eord(a.effort) - eord(b.effort); });
    var base = baselinePoint();
    sel.innerHTML = pool.map(function (p) {
      return '<option value="' + esc(p.key) + '"' + (base && p.key === base.key ? " selected" : "") + ">" + esc(shortLabel(p)) + "</option>";
    }).join("");
  }
  function drawMultiples() {
    var svg = document.getElementById("xpl-c5");
    if (!svg) return;
    var base = baselinePoint();
    var note = svg.parentElement.querySelector("[data-note]");
    var all = visPoints();
    var rows = base ? all.filter(function (p) { return p.cost != null && p.time != null && p.score != null; }) : [];
    if (note) {
      note.textContent = base
        ? "Baseline: " + shortLabel(base) + " = 1× on all three measures (" + fmt$(base.cost) + "/task, " + fmtT(base.time) + "/task, " + scoreWord() + " " + fmtN(base.score, 1) + "). " + rows.length + " of " + all.length + " configurations have all three measures."
        : "No configuration with cost, time and score is visible — the multiples view needs all three.";
    }
    renderMultTable(base, rows);
    if (!base || !rows.length) { svg.innerHTML = ""; return; }
    var pts = rows.map(function (p) {
      return { p: p, cx: p.cost / base.cost, ty: p.time / base.time, sx: p.score / base.score };
    });
    var xs = pts.map(function (t) { return t.cx; }), ys = pts.map(function (t) { return t.ty; });
    var xlo = Math.min(0.8, Math.min.apply(null, xs) / 1.4), xhi = Math.max.apply(null, xs) * 1.5;
    var ylo = Math.min(0.8, Math.min.apply(null, ys) / 1.4), yhi = Math.max.apply(null, ys) * 1.5;
    var X = function (v) { return ML + logPos(v, xlo, xhi) * PW; };
    var Y = function (v) { return MT + (1 - logPos(v, ylo, yhi)) * PH; };
    var g = "";
    [1, 3, 10, 30, 100, 300, 1000].filter(function (v) { return v >= xlo && v <= xhi; }).forEach(function (v) {
      g += '<line x1="' + X(v) + '" y1="' + MT + '" x2="' + X(v) + '" y2="' + (MT + PH) + '" stroke="#30363d" stroke-width="0.5" opacity="0.6"/><text x="' + X(v) + '" y="' + (H - MB + 16) + '" font-size="9.5" fill="#8b949e" text-anchor="middle">' + v + "×</text>";
    });
    [1, 3, 10, 30, 100, 300].filter(function (v) { return v >= ylo && v <= yhi; }).forEach(function (v) {
      g += '<line x1="' + ML + '" y1="' + Y(v) + '" x2="' + (ML + PW) + '" y2="' + Y(v) + '" stroke="#30363d" stroke-width="0.5" opacity="0.6"/><text x="' + (ML - 5) + '" y="' + (Y(v) + 3) + '" font-size="9.5" fill="#8b949e" text-anchor="end">' + v + "×</text>";
    });
    if (xlo < 1 && xhi > 1) g += '<line x1="' + X(1) + '" y1="' + MT + '" x2="' + X(1) + '" y2="' + (MT + PH) + '" stroke="#58a6ff" stroke-width="1" opacity="0.55"/>';
    if (ylo < 1 && yhi > 1) g += '<line x1="' + ML + '" y1="' + Y(1) + '" x2="' + (ML + PW) + '" y2="' + Y(1) + '" stroke="#58a6ff" stroke-width="1" opacity="0.55"/>';
    g += '<line x1="' + ML + '" y1="' + (MT + PH) + '" x2="' + (ML + PW) + '" y2="' + (MT + PH) + '" stroke="#30363d"/><line x1="' + ML + '" y1="' + MT + '" x2="' + ML + '" y2="' + (MT + PH) + '" stroke="#30363d"/>';
    g += '<text x="' + (ML + PW / 2) + '" y="' + (H - 4) + '" font-size="10.5" fill="#8b949e" text-anchor="middle">cost multiple of the baseline (log scale)</text>';
    g += '<text x="12" y="' + (MT + PH / 2) + '" font-size="10.5" fill="#8b949e" text-anchor="middle" transform="rotate(-90 12 ' + (MT + PH / 2) + ')">duration multiple of the baseline (lower = faster)</text>';
    pts.forEach(function (t) {
      var p = t.p, cx = X(t.cx), cy = Y(t.ty), col = famColor[p.family];
      var hollow = p.benchmark && p.benchmark.indexOf("AA") !== 0;
      var tip = shortLabel(p) + " — costs " + fmtN(t.cx, 2) + "× baseline (" + fmt$(p.cost) + "/task), takes " + fmtN(t.ty, 2) + "× as long (" + fmtT(p.time) + "/task), scores " + fmtN(t.sx, 2) + "× (" + scoreWord() + " " + fmtN(p.score, 1) + ")";
      g += '<circle class="xpt" data-key="' + esc(p.key) + '" cx="' + cx.toFixed(1) + '" cy="' + cy.toFixed(1) + '" r="5.5" fill="' + col + '" fill-opacity="' + (hollow ? 0.25 : 0.9) + '" stroke="' + (S.sel === p.key ? "#fff" : col) + '" stroke-width="' + (S.sel === p.key ? 2 : 1.5) + '"' + (hollow ? ' stroke-dasharray="2.5 1.8"' : "") + "><title>" + esc(tip) + "</title></circle>";
      if (p.key === base.key) g += '<circle cx="' + cx.toFixed(1) + '" cy="' + cy.toFixed(1) + '" r="10" fill="none" stroke="#58a6ff" stroke-width="1.5"/>';
      if (p.frontierCostTime || S.sel === p.key || p.key === base.key) {
        var anchorEnd = cx > ML + PW * 0.78;
        g += '<text data-key="' + esc(p.key) + '" x="' + (anchorEnd ? cx - 9 : cx + 9) + '" y="' + (cy + 3) + '" font-size="8.6" fill="#c9d1d9" text-anchor="' + (anchorEnd ? "end" : "start") + '" style="cursor:pointer">' + esc(shortLabel(p)) + "</text>";
      }
    });
    svg.innerHTML = g;
    svg.querySelectorAll("[data-key]").forEach(function (el) {
      el.addEventListener("click", function () { selectPoint(el.getAttribute("data-key")); });
    });
  }
  function renderMultTable(base, rows) {
    var tb = document.getElementById("xpl-mtrows");
    if (!tb) return;
    if (!base) { tb.innerHTML = ""; return; }
    var sorted = rows.slice().sort(function (a, b) { return a.cost / base.cost - b.cost / base.cost; });
    tb.innerHTML = sorted.map(function (p) {
      return '<tr data-sel="' + esc(p.key) + '" data-key="' + esc(p.key) + '"><td><strong>' + esc(p.family) + "</strong></td><td>" + esc(p.effort || "—") + '</td><td class="num">' + fmtN(p.score, 1) + '</td><td class="num">' + fmtN(p.score / base.score, 2) + '×</td><td class="num xpl-costcol">' + fmt$(p.cost) + '</td><td class="num xpl-costcol">' + fmtN(p.cost / base.cost, 2) + '×</td><td class="num">' + fmtT(p.time) + '</td><td class="num">' + fmtN(p.time / base.time, 2) + '×</td><td>' + esc(p.benchmark || "—") + "</td></tr>";
    }).join("");
    tb.querySelectorAll("tr").forEach(function (tr) {
      tr.addEventListener("click", function () { selectPoint(tr.getAttribute("data-key")); });
    });
    markSelectedRow();
  }
  // --- chart 6: scores only, incl. configurations with no published cost ---
  function drawScores() {
    var svg = document.getElementById("xpl-c6");
    if (!svg) return;
    var all = visPoints();
    var rows = all.filter(function (p) { return p.score != null; }).sort(function (a, b) { return b.score - a.score; });
    var note = svg.parentElement.querySelector("[data-note]");
    var top = rows.slice(0, 40);
    if (note) {
      note.textContent = "Top " + top.length + " of " + rows.length + " scored configurations (of " + all.length + " visible). Score-only view: configurations with no published cost belong here even though no cost chart can plot them.";
    }
    if (!top.length) { svg.innerHTML = ""; return; }
    var rowH = 17, topPad = 8, botPad = 22;
    var hgt = topPad + top.length * rowH + botPad;
    svg.setAttribute("viewBox", "0 0 680 " + hgt);
    var labelW = 218, plotW = 680 - labelW - 52;
    var maxScore = Math.max.apply(null, top.map(function (p) { return p.score; })) * 1.05;
    var g = "";
    top.forEach(function (p, i) {
      var y = topPad + i * rowH;
      var w = Math.max(2, (p.score / maxScore) * plotW);
      var col = famColor[p.family];
      var label = shortLabel(p) + (p.cost == null ? " (cost n/a)" : "");
      g += '<text data-key="' + esc(p.key) + '" x="' + (labelW - 6) + '" y="' + (y + 11) + '" font-size="9.5" fill="#c9d1d9" text-anchor="end" style="cursor:pointer">' + esc(label) + "</text>";
      g += '<rect data-key="' + esc(p.key) + '" x="' + labelW + '" y="' + (y + 2) + '" width="' + w.toFixed(1) + '" height="11" fill="' + col + '" fill-opacity="' + (S.sel === p.key ? 1 : 0.75) + '" style="cursor:pointer"><title>' + esc(shortLabel(p) + " — " + scoreWord() + " " + fmtN(p.score, 1) + " · " + fmt$(p.cost) + "/task · " + fmtT(p.time) + "/task") + "</title></rect>";
      g += '<text x="' + (labelW + w + 5) + '" y="' + (y + 11) + '" font-size="9.5" fill="#8b949e">' + fmtN(p.score, 1) + "</text>";
    });
    svg.innerHTML = g;
    svg.querySelectorAll("[data-key]").forEach(function (el) {
      el.addEventListener("click", function () { selectPoint(el.getAttribute("data-key")); });
    });
  }
  // --- hidden tray (reference design's hide-a-configuration flow) ---
  function renderUnpriced() {
    // #1400 remainder: visible scored configurations with no published
    // cost cannot appear on the cost charts; list them under the charts
    // as real buttons that open the existing detail card via
    // selectPoint — the same flow as clicking a dot or a table row.
    // visPoints() already applies every filter and the hidden set.
    var box = document.getElementById("xpl-unpriced");
    if (!box) return;
    var rows = unpricedScoredPoints(visPoints());
    box.innerHTML = "";
    if (!rows.length) {
      var none = document.createElement("p");
      none.className = "muted";
      none.textContent = "No visible scored configurations are missing a published cost.";
      box.appendChild(none);
      return;
    }
    rows.forEach(function (p) {
      var b = document.createElement("button");
      b.type = "button";
      b.className = "xpl-ctl-btn";
      b.setAttribute("data-unpriced", p.key);
      b.textContent = (p.family || p.key) + (p.effort ? " \u2014 " + p.effort : "") + " \u00b7 " + scoreWord() + " " + fmtN(p.score, 1) + " \u00b7 no published cost";
      b.addEventListener("click", function () { selectPoint(p.key); });
      box.appendChild(b);
    });
  }
  function renderTray() {
    var tray = document.getElementById("xpl-tray");
    if (!tray) return;
    var chips = document.getElementById("xpl-traychips");
    if (!S.hidden.length) { tray.hidden = true; chips.innerHTML = ""; return; }
    tray.hidden = false;
    chips.innerHTML = "";
    S.hidden.forEach(function (key) {
      var p = POINTS.find(function (x) { return x.key === key; });
      if (!p) return;
      var s = document.createElement("span");
      s.className = "hchip";
      s.textContent = shortLabel(p);
      var b = document.createElement("button");
      b.type = "button";
      b.textContent = "×";
      b.title = "Bring back";
      b.addEventListener("click", function () {
        S.hidden = S.hidden.filter(function (h) { return h !== key; });
        render();
      });
      s.appendChild(b);
      chips.appendChild(s);
    });
  }
  function renderLegend() {
    var lg = document.getElementById("xpl-legend");
    if (!lg) return;
    lg.innerHTML = "";
    famList.forEach(function (fam) {
      var b = document.createElement("button");
      b.type = "button";
      b.className = "xchip" + (S.off[fam] ? " off" : "");
      b.innerHTML = '<span class="dot" style="background:' + famColor[fam] + '"></span>' + esc(fam);
      b.addEventListener("click", function () {
        if (S.off[fam]) delete S.off[fam]; else S.off[fam] = true;
        render();
      });
      lg.appendChild(b);
    });
  }
  function showDetail(html) {
    var box = document.getElementById("xpl-detail");
    if (!box) return;
    if (!html) { box.style.display = "none"; box.innerHTML = ""; return; }
    box.innerHTML = html;
    box.style.display = "block";
  }
  function selectPoint(key) {
    S.sel = key;
    var p = POINTS.find(function (x) { return x.key === key; });
    if (!p) return;
    var beaten = p.beatenBy && p.beatenBy.length
      ? p.beatenBy.slice(0, 4).join("; ") + (p.beatenBy.length > 4 ? " (+" + (p.beatenBy.length - 4) + " more)" : "")
      : "nothing on all three axes — it is on the frontier";
    var budgets = p.cost
      ? "<br>Tasks per budget at this cost: $60 → " + fmtInt(tasksAt(p.cost, 60)) + " · $90 → " + fmtInt(tasksAt(p.cost, 90)) + " · $120 → " + fmtInt(tasksAt(p.cost, 120)) + " · $180 → " + fmtInt(tasksAt(p.cost, 180))
      : "";
    var cap = goCapFor(p);
    var goLine = cap && p.cost
      ? '<br><span class="muted">Published OpenCode Go cap for this model: $' + cap + "/mo → ~" + fmtInt(Math.floor(cap / p.cost)) + " tasks at this AA cost (docs reading — the fleet's workspaces appear to meter pooled).</span>"
      : "";
    var caiLine = "";
    if (p.harness != null || p.components) {
      var pct = function (v) { return v == null ? "—" : fmtN(v * 100, 1) + "%"; };
      caiLine = "<br>" +
        (p.harness ? "Harness: " + esc(p.harness) + " · " : "") +
        (p.tokensPerTask != null ? fmtInt(p.tokensPerTask) + " tokens/task" : "") +
        (p.components ? "<br>Components: DeepSWE " + pct(p.components.deepSwe) + " · Terminal-Bench " + pct(p.components.terminalBench) + " · SWE-Atlas-QnA " + pct(p.components.sweAtlas) : "");
    }
    showDetail(
      "<strong>" + esc(p.family) + " — " + esc(p.effort || "—") + "</strong> <span class='muted'>" + esc(p.creator) + "</span><br>" +
      scoreWord() + " " + fmtN(p.score, 1) + " · " + fmt$(p.cost) + "/task · " + fmtT(p.time) + " per task" +
      (p.ttft != null ? " · TTFT " + fmtT(p.ttft) : "") + (p.tps != null ? " · " + p.tps + " tok/s" : "") + budgets + "<br>" +
      '<span class="muted">Beaten on score + cost + time at once by: ' + esc(beaten) + "</span>" + caiLine + goLine +
      '<br><button type="button" id="xpl-hide" class="xpl-ctl-btn">Hide this configuration</button>',
    );
    var hideBtn = document.getElementById("xpl-hide");
    if (hideBtn) {
      hideBtn.addEventListener("click", function () {
        if (S.hidden.indexOf(p.key) < 0) S.hidden.push(p.key);
        S.sel = null;
        showDetail(null);
        render();
      });
    }
    drawCharts();
    markSelectedRow();
  }
  function selectCatalog(id) {
    S.sel = id;
    var e = (CATALOG || []).find(function (x) { return x.id === id; });
    if (!e) return;
    showDetail(
      "<strong>" + esc(e.name) + "</strong> <span class='muted'><code>" + esc(e.id) + "</code></span><br>" +
      (e.plan ? "Plan: <strong>" + esc(e.plan) + "</strong>" + (e.multiplier ? " — " + e.multiplier + "× list value" : "") + "<br>" : "") +
      (e.planNote ? '<span class="muted">' + esc(e.planNote) + "</span><br>" : "") +
      "Pool seats: " + (e.routes.length ? e.routes.map(function (r) { return "<code>" + esc(r) + "</code>"; }).join(" ") : '<span class="muted">none — no route sends it work</span>') +
      (e.goCapUsd != null && e.aaCostPerTask ? '<br><span class="muted">Published OpenCode Go cap: $' + e.goCapUsd + "/mo → ~" + fmtInt(Math.floor(e.goCapUsd / e.aaCostPerTask)) + " AA tasks at the measured cost (docs reading — the fleet's workspaces appear to meter pooled).</span>" : "") +
      (e.disabled ? '<br><span class="muted">Provider is disabled in the fleet config — catalogued but not callable.</span>' : "") +
      (function () {
        var proj = project(e);
        if (!proj.band) return "";
        var P = S.pj;
        return '<br><span class="muted">Your projected task (ESTIMATED, NOT MEASURED): ' + P.turns + " turns × " + fmtInt(Number(P.inputTokens) || 0) + " input + " + fmtInt(Number(P.answerTokens) || 0) + " answer + " + fmtInt(Number(P.thinkingTokens) || 0) + " thinking tokens/turn · " + P.cacheReadPct + "% cache reads · rate window: " + esc(proj.band.window || "flat") + " (" + fmt$(proj.band.input) + " in / " + fmt$(proj.band.output) + " out per 1M) → " + (proj.cost != null ? "~" + fmt$(proj.cost) + "/task" : "cost unknown — a required rate is not recorded") + (proj.time != null ? " · ~" + fmtT(proj.time) + "/task" : "") + ".</span>";
      })(),
    );
    markSelectedRow();
  }
  function markSelectedRow() {
    document.querySelectorAll(".xpl-table tr").forEach(function (tr) {
      tr.style.background = tr.getAttribute("data-sel") === S.sel ? "#1f6feb22" : "";
    });
  }
  // --- Pareto frontier tables (Site 1's Pareto frontiers tab) ------------
  // Each list keeps variants not dominated on its named dimensions;
  // percentile ranks use the current filtered population (100% = best).
  function pfBind(tb) {
    if (!tb) return;
    tb.querySelectorAll("tr").forEach(function (tr) {
      tr.addEventListener("click", function () { selectPoint(tr.getAttribute("data-key")); });
    });
  }
  function pfMetricCell(v, pct, fmt) {
    return v == null ? "—" : fmt(v) + (pct != null ? ' <span class="muted">· ' + pct + "%</span>" : "");
  }
  function pfNameCell(p) {
    return '<td><strong>' + esc(p.family) + '</strong> <span class="muted">' + esc(p.effort || "") + (p.harness ? " · " + esc(p.harness) : "") + "</span></td>";
  }
  function renderPareto() {
    var rows = visPoints();
    var scores = rows.map(function (p) { return p.score; });
    var times = rows.map(function (p) { return p.time; });
    var costs = rows.map(function (p) { return p.cost; });
    function stdRow(p) {
      return '<tr data-sel="' + esc(p.key) + '" data-key="' + esc(p.key) + '">' + pfNameCell(p) +
        '<td class="num">' + pfMetricCell(p.score, pctRank(scores, p.score, true), function (v) { return fmtN(v, 1); }) + "</td>" +
        '<td class="num">' + pfMetricCell(p.time, pctRank(times, p.time, false), fmtT) + "</td>" +
        '<td class="num xpl-costcol">' + pfMetricCell(p.cost, pctRank(costs, p.cost, false), fmt$) + "</td></tr>";
    }
    var gScore = function (p) { return p.score; }, gTime = function (p) { return p.time; }, gCost = function (p) { return p.cost; };
    var ssFrontier = paretoSet(rows, [{ get: gScore, better: "high" }, { get: gTime, better: "low" }]);
    var byScore = ssFrontier.slice().sort(function (a, b) { return cmpDesc(a.score, b.score) || cmpAsc(a.time, b.time); });
    var bySpeed = ssFrontier.slice().sort(function (a, b) { return cmpAsc(a.time, b.time) || cmpDesc(a.score, b.score); });
    var cut = Number(S.pfCut || 50);
    var three = paretoSet(rows, [{ get: gScore, better: "high" }, { get: gCost, better: "low" }, { get: gTime, better: "low" }])
      .filter(function (p) {
        if (!cut) return true;
        var ps = pctRank(scores, p.score, true), pt = pctRank(times, p.time, false);
        return ps != null && pt != null && ps >= cut && pt >= cut;
      })
      .sort(function (a, b) { return cmpAsc(a.cost, b.cost) || cmpDesc(a.score, b.score); });
    var psFrontier = paretoSet(rows, [{ get: gCost, better: "low" }, { get: gTime, better: "low" }])
      .sort(function (a, b) { return cmpAsc(a.cost, b.cost) || cmpAsc(a.time, b.time); });
    var tb;
    tb = document.getElementById("xpl-pf-score"); if (tb) { tb.innerHTML = byScore.map(stdRow).join(""); pfBind(tb); }
    tb = document.getElementById("xpl-pf-speed"); if (tb) { tb.innerHTML = bySpeed.map(stdRow).join(""); pfBind(tb); }
    tb = document.getElementById("xpl-pf-price"); if (tb) { tb.innerHTML = three.map(stdRow).join(""); pfBind(tb); }
    tb = document.getElementById("xpl-pf-ps");
    if (tb) {
      tb.innerHTML = psFrontier.map(function (p) {
        return '<tr data-sel="' + esc(p.key) + '" data-key="' + esc(p.key) + '">' + pfNameCell(p) +
          '<td class="num">' + pfMetricCell(p.time, pctRank(times, p.time, false), fmtT) + "</td>" +
          '<td class="num xpl-costcol">' + pfMetricCell(p.cost, pctRank(costs, p.cost, false), fmt$) + "</td>" +
          '<td class="num">' + (p.score == null ? "—" : fmtN(p.score, 1)) + "</td></tr>";
      }).join("");
      pfBind(tb);
    }
    var note = document.getElementById("xpl-pf-count");
    if (note) {
      note.textContent = "Score–speed frontier: " + ssFrontier.length + " variants · three-way frontier" +
        (cut ? " (score and speed at or above the " + cut + "th percentile)" : " (no percentile cutoff)") + ": " + three.length +
        " · price–speed trade-offs: " + psFrontier.length + ". Missing measures are excluded only from a frontier that needs them.";
    }
    markSelectedRow();
  }
  // --- Recommendations (Site 1's shortlist builder) ------------------------
  // Score bands are non-overlapping, from the visible population, before
  // tradeoff limits. Within a band: absolute limits first, then relative
  // price/speed limits against the cheapest and fastest near-top models;
  // survivors rank by (relative time)^speed-weight x (relative
  // price)^(1-speed-weight); scores inside the window break ties.
  function computeRecs() {
    return computeRecommendations(visPoints(), S.rec, S.bandCount, scoreWord());
  }
  function bandScatter(band) {
    var pts = band.members.filter(function (p) { return p.cost != null && p.time != null; });
    if (pts.length < 2) return "";
    var pickKeys = {};
    band.picks.forEach(function (r) { pickKeys[r.p.key] = true; });
    var bw = 330, bh = 180, ml = 44, mr = 8, mt = 8, mb = 24, pw = bw - ml - mr, ph = bh - mt - mb;
    var xs = pts.map(function (p) { return p.cost; }), ys = pts.map(function (p) { return p.time; });
    var xlo = Math.pow(10, Math.floor(Math.log10(Math.min.apply(null, xs)) * 2) / 2) / 1.4, xhi = Math.max.apply(null, xs) * 1.5;
    var ylo = Math.pow(10, Math.floor(Math.log10(Math.min.apply(null, ys)) * 2) / 2) / 1.4, yhi = Math.max.apply(null, ys) * 1.5;
    // Faster is DOWN: screen y grows with time (no inversion).
    var X = function (v) { return ml + logPos(v, xlo, xhi) * pw; };
    var Y = function (v) { return mt + logPos(v, ylo, yhi) * ph; };
    var g = '<svg viewBox="0 0 ' + bw + " " + bh + '" role="img" aria-label="Price versus speed in this score band" style="max-width:340px;width:100%">';
    g += '<line x1="' + ml + '" y1="' + (mt + ph) + '" x2="' + (ml + pw) + '" y2="' + (mt + ph) + '" stroke="#30363d"/><line x1="' + ml + '" y1="' + mt + '" x2="' + ml + '" y2="' + (mt + ph) + '" stroke="#30363d"/>';
    g += '<text x="' + (ml + pw / 2) + '" y="' + (bh - 4) + '" font-size="9" fill="#8b949e" text-anchor="middle">USD / task (log) — cheaper is left</text>';
    g += '<text x="10" y="' + (mt + ph / 2) + '" font-size="9" fill="#8b949e" text-anchor="middle" transform="rotate(-90 10 ' + (mt + ph / 2) + ')">time — faster is down</text>';
    pts.forEach(function (p) {
      var on = !!pickKeys[p.key], col = famColor[p.family];
      g += '<circle class="xpt" data-key="' + esc(p.key) + '" cx="' + X(p.cost).toFixed(1) + '" cy="' + Y(p.time).toFixed(1) + '" r="' + (on ? 5 : 3.6) + '" fill="' + col + '" fill-opacity="' + (on ? 0.95 : 0.4) + '" stroke="' + (on ? "#fff" : col) + '" stroke-width="' + (on ? 1.6 : 0.8) + '"><title>' + esc(shortLabel(p) + " — " + fmt$(p.cost) + "/task · " + fmtT(p.time)) + "</title></circle>";
    });
    return g + "</svg>";
  }
  function renderRecs() {
    var wrap = document.getElementById("xpl-recs");
    if (!wrap) return;
    var out = computeRecs();
    if (out.empty) { wrap.innerHTML = '<p class="muted">' + esc(out.empty) + "</p>"; return; }
    var R = S.rec;
    var summary = document.getElementById("xpl-rec-summary");
    var totalPicks = out.bands.reduce(function (a, b) { return a + b.picks.length; }, 0);
    var labs = {};
    out.bands.forEach(function (b) { b.picks.forEach(function (r) { labs[r.p.creator] = true; }); });
    if (summary) {
      summary.textContent = totalPicks + " choices across " + out.bands.length + " score bands · " + Object.keys(labs).length + " labs. Near-top: " + R.allow +
        " points · Speed priority: " + R.speedPriority + "% · At most " + R.slowAllow + "× slower · Price allowance: +" + R.priceAllow + "% · " +
        (R.diversity === "lab" ? "One per lab" : R.diversity === "labsfirst" ? "Different labs first" : "No lab grouping") + ".";
    }
    wrap.innerHTML = out.bands.map(function (band, bi) {
      var ref = band.ref;
      var cards = band.picks.map(function (r, ri) {
        var p = r.p;
        var vsRef = "";
        if (ref && ref.key !== p.key) {
          var dt = ref.time ? Math.round((1 - p.time / ref.time) * 1000) / 10 : null;
          var dc = ref.cost ? Math.round((1 - p.cost / ref.cost) * 1000) / 10 : null;
          vsRef = '<br><span class="muted">vs band-best ' + esc(shortLabel(ref)) + ': ' +
            (dt != null ? (dt >= 0 ? dt + "% less time" : Math.abs(dt) + "% more time") : "") +
            (dt != null && dc != null ? " · " : "") +
            (dc != null ? (dc >= 0 ? dc + "% less cost" : Math.abs(dc) + "% more cost") : "") + "</span>";
        } else if (ref) {
          vsRef = '<br><span class="muted">The top-scoring reference for this band.</span>';
        }
        var bestFactor = band.picks.length ? band.picks[0].factor : 1;
        return '<div class="card"><span class="badge ' + (ri === 0 ? "open" : "src") + '">' + (ri === 0 ? "Best fit" : p.creator === band.picks[0].p.creator ? "Alternative" : "Alternative lab") + "</span> " +
          '<button type="button" class="xpl-linkbtn" data-pick="' + esc(p.key) + '"><strong>' + esc(shortLabel(p)) + "</strong></button>" +
          '<span class="muted"> · ' + esc(p.creator) + (p.harness ? " · " + esc(p.harness) : "") + "</span><br>" +
          scoreWord() + " " + fmtN(p.score, 1) + " · " + fmt$(p.cost) + "/task · " + fmtT(p.time) + "/task · " +
          fmtN(band.best - p.score, 1) + " below band best · " + fmtN(r.factor / bestFactor, 2) + "× best tradeoff" + vsRef + "</div>";
      }).join("");
      var counts = {};
      band.members.forEach(function (p) { var d = band.decision[p.key] || "Not selected"; counts[d] = (counts[d] || 0) + 1; });
      var whyRows = band.members.slice().sort(function (a, b) {
        var ra = band.decision[a.key] === "Recommended" ? 0 : 1, rb = band.decision[b.key] === "Recommended" ? 0 : 1;
        return ra - rb || cmpDesc(a.score, b.score);
      }).map(function (p) {
        return '<tr data-key="' + esc(p.key) + '"><td><strong>' + esc(p.family) + '</strong> <span class="muted">' + esc(p.effort || "") + '</span></td><td>' + esc(p.creator) + "</td><td>" + esc(band.decision[p.key] || "Not selected") + "</td></tr>";
      }).join("");
      var whyCounts = Object.keys(counts).map(function (k) { return counts[k] + " · " + k; }).join(" &nbsp;·&nbsp; ");
      return "<h3>" + esc(band.label) + " · " + band.members.length + " variants</h3>" +
        '<p class="muted">Require score ≥ ' + fmtN(band.floor, 1) + " (band best " + fmtN(band.best, 1) + ", allowance " + R.allow + " points)" +
        (band.priceCeil != null ? " · Relative price ceiling " + fmt$(band.priceCeil) + " · Relative time ceiling " + fmtT(band.timeCeil) : " · no candidate carries all three measures") + "</p>" +
        bandScatter(band) +
        '<div class="board">' + (cards || '<p class="muted">No recommendation survives the limits of this band.</p>') + "</div>" +
        '<details><summary>Why these choices?</summary><p class="muted">' + whyCounts + '</p><div class="tblwrap"><table class="xpl-table"><caption>Why these choices — decisions by model and effort</caption><thead><tr><th scope="col">Model / effort</th><th scope="col">Lab</th><th scope="col">Decision</th></tr></thead><tbody>' + whyRows + "</tbody></table></div></details>";
    }).join("");
    wrap.querySelectorAll("[data-pick]").forEach(function (b) {
      b.addEventListener("click", function () { selectPoint(b.getAttribute("data-pick")); });
    });
    wrap.querySelectorAll("tbody tr").forEach(function (tr) {
      tr.addEventListener("click", function () { selectPoint(tr.getAttribute("data-key")); });
    });
    wrap.querySelectorAll("svg [data-key]").forEach(function (el) {
      el.addEventListener("click", function () { selectPoint(el.getAttribute("data-key")); });
    });
    markSelectedRow();
    // Recommendation inputs redraw #xpl-recs directly, without
    // going through render() (Codex P2, PR #4154) — re-enhance here
    // so the band charts keep their expand/zoom/pan controls.
    enhanceCharts();
  }
  // --- Band comparisons (Site 1's six banded shortlists) -------------------
  // Near-leader windows at 1x: score within 2 points, time within 10%,
  // price within 25% of the band leader; the scale select multiplies all
  // three. Lane charts visualize the same near-leader choices as the
  // comparison tables; they do not recalculate band membership.
  // logEdges/scoreEdges/bandIndex come from the aa-explorer-computations.js import above (#7562): no inline copies here - an inline definition would shadow the tested module implementation.
  function chipFor(p, leader) {
    var notes = [];
    if (leader && leader.key !== p.key) {
      var better = [];
      if (p.score != null && leader.score != null && p.score > leader.score) better.push("higher score");
      if (p.cost != null && leader.cost != null && p.cost < leader.cost) better.push("lower price");
      if (p.time != null && leader.time != null && p.time < leader.time) better.push("faster");
      if (better.length) notes.push("Better at: " + better.join(", "));
    }
    return '<button type="button" class="xchip" data-pick="' + esc(p.key) + '">' + esc(shortLabel(p)) + " — " + fmtN(p.score, 1) + " · " + fmt$(p.cost) + " · " + fmtT(p.time) + (notes.length ? ' <span class="muted">(' + esc(notes.join("; ")) + ")</span>" : "") + "</button>";
  }
  function bandRangeLine(members) {
    var withScore = members.filter(function (p) { return p.score != null; });
    var withTime = members.filter(function (p) { return p.time != null; });
    var parts = [];
    if (withScore.length) {
      var sMin = withScore.reduce(function (a, b) { return b.score < a.score ? b : a; });
      var sMax = withScore.reduce(function (a, b) { return b.score > a.score ? b : a; });
      parts.push("score " + fmtN(sMin.score, 1) + " (" + shortLabel(sMin) + ") – " + fmtN(sMax.score, 1) + " (" + shortLabel(sMax) + ")");
    }
    if (withTime.length) {
      var tMin = withTime.reduce(function (a, b) { return b.time < a.time ? b : a; });
      var tMax = withTime.reduce(function (a, b) { return b.time > a.time ? b : a; });
      parts.push("time " + fmtT(tMin.time) + " (" + shortLabel(tMin) + ") – " + fmtT(tMax.time) + " (" + shortLabel(tMax) + ")");
    }
    return parts.join(" · ");
  }
  function bandLaneChart(title, metric, axisLabel, bands) {
    var lanes = bands.filter(function (b) { return b.choices && b.choices.length; });
    var values = [];
    lanes.forEach(function (b) {
      b.members.forEach(function (p) {
        var v = p[metric];
        if (v != null && isFinite(v) && (metric === "score" || v > 0)) values.push(v);
      });
    });
    if (!values.length) return "";
    var lo = Math.min.apply(null, values), hi = Math.max.apply(null, values);
    if (lo === hi) {
      if (metric === "score") { lo -= 1; hi += 1; }
      else { lo /= 1.4; hi *= 1.4; }
    }
    var logarithmic = metric !== "score";
    var logLo = logarithmic ? Math.log(lo) : lo;
    var logHi = logarithmic ? Math.log(hi) : hi;
    var width = 760, left = 232, right = 16, top = 28, row = 30, bottom = 30;
    var plotWidth = width - left - right;
    var height = top + lanes.length * row + bottom;
    function x(v) {
      var n = logarithmic ? Math.log(v) : v;
      return left + ((n - logLo) / (logHi - logLo)) * plotWidth;
    }
    function valueLabel(v) {
      return metric === "cost" ? fmt$(v) : metric === "time" ? fmtT(v) : fmtN(v, 1);
    }
    var svg = '<svg class="xpl-band-lanes" viewBox="0 0 ' + width + " " + height + '" role="img" aria-label="' + esc(title + " lane chart; " + axisLabel) + '" style="max-width:760px;width:100%;height:auto">';
    svg += '<text x="' + left + '" y="13" font-size="10" fill="#8b949e">' + esc(axisLabel) + (logarithmic ? " (log scale)" : " (linear scale)") + "</text>";
    for (var ti = 0; ti <= 4; ti++) {
      var fraction = ti / 4;
      var tv = logarithmic ? Math.exp(logLo + (logHi - logLo) * fraction) : lo + (hi - lo) * fraction;
      var tx = left + plotWidth * fraction;
      svg += '<line x1="' + tx.toFixed(1) + '" y1="' + (top - 4) + '" x2="' + tx.toFixed(1) + '" y2="' + (height - bottom + 2) + '" stroke="#30363d" stroke-dasharray="2 3"/>';
      svg += '<text x="' + tx.toFixed(1) + '" y="' + (height - 8) + '" font-size="9" fill="#8b949e" text-anchor="middle">' + esc(valueLabel(tv)) + "</text>";
    }
    lanes.forEach(function (band, bi) {
      var cy = top + bi * row + row / 2;
      var label = band.label.length > 38 ? band.label.slice(0, 35) + "…" : band.label;
      svg += '<text x="8" y="' + (cy + 3) + '" font-size="9" fill="#c9d1d9">' + esc(label) + "</text>";
      svg += '<line x1="' + left + '" y1="' + cy + '" x2="' + (width - right) + '" y2="' + cy + '" stroke="#30363d"/>';
      band.choices.forEach(function (p, pi) {
        var v = p[metric];
        if (v == null || !isFinite(v) || (logarithmic && v <= 0)) return;
        var leader = band.leader || band.choices[0];
        var isLeader = leader && leader.key === p.key;
        var py = cy + ((pi % 5) - 2) * 2.5;
        var color = famColor[p.family] || "#8b949e";
        var desc = shortLabel(p) + " — " + axisLabel + " " + valueLabel(v);
        const pointX = x(v).toFixed(1);
        const pointY = py.toFixed(1);
        svg += '<g class="xpt-control" data-pick="' + esc(p.key) + '" tabindex="0" role="button" aria-label="' + esc("Select " + desc) + '"><circle class="xpt-hit" cx="' + pointX + '" cy="' + pointY + '" r="40"/><circle class="xpt" cx="' + pointX + '" cy="' + pointY + '" r="' + (isLeader ? 5 : 3.8) + '" fill="' + color + '" fill-opacity="' + (isLeader ? 0.95 : 0.65) + '" stroke="' + (isLeader ? "#fff" : color) + '" stroke-width="' + (isLeader ? 1.5 : 0.8) + '"/><title>' + esc(desc + (isLeader ? " — band leader" : " — near-leader")) + "</title></g>";
      });
    });
    return '<div class="xpl-band-lane-wrap">' + svg + "</svg></div>";
  }
  function bandGroup(title, blurb, bands, laneMetric, axisLabel) {
    var rowsHtml = bands.map(function (b) {
      var leader = b.choices.length ? b.choices[0] : null;
      return "<tr><th scope=\\\"row\\\"><strong>" + esc(b.label) + '</strong><br><span class="muted">' + b.choices.length + " near-leading · " + esc(bandRangeLine(b.members)) + "</span></th><td>" +
        (b.choices.length ? b.choices.map(function (p) { return chipFor(p, leader); }).join(" ") : '<span class="muted">No model meets the window of this band.</span>') + "</td></tr>";
    }).join("");
    var chart = laneMetric ? bandLaneChart(title, laneMetric, axisLabel, bands) : "";
    return "<h3>" + esc(title) + '</h3><p class="muted">' + esc(blurb) + '</p>' + chart + '<div class="tblwrap"><table class="xpl-table"><caption>' + esc(title) + '</caption><tbody>' + rowsHtml + "</tbody></table></div>";
  }
  function renderBandComparisons() {
    var wrap = document.getElementById("xpl-bandgroups");
    if (!wrap) return;
    var rows = visPoints();
    var n = Math.max(2, Math.min(8, Number(S.bandCount) || 3));
    var scale = Number(S.bandScale) || 1;
    var scoreWin = 2 * scale, timeWin = 0.10 * scale, priceWin = 0.25 * scale;
    var costEdges = logEdges(rows.map(function (p) { return p.cost; }), n);
    var timeEdges = logEdges(rows.map(function (p) { return p.time; }), n);
    var sEdges = scoreEdges(rows, n);
    function groupBy(edges, get) {
      var out = [];
      for (var i = 0; i < edges.length - 1; i++) out.push([]);
      rows.forEach(function (p) {
        var v = get(p);
        if (v == null) return;
        var idx = bandIndex(v, edges);
        if (idx >= 0) out[idx].push(p);
      });
      return out;
    }
    function costLabel(i) { return (i === 0 ? "≥ " : "> ") + fmt$(costEdges[i]) + " · ≤ " + fmt$(costEdges[i + 1]); }
    function timeLabel(i) { return fmtT(timeEdges[i]) + " – " + fmtT(timeEdges[i + 1]); }
    function scoreLabel(i) { return fmtN(sEdges[i], 1) + (i === sEdges.length - 2 ? " – ≤ " : " – < ") + fmtN(sEdges[i + 1], 1); }
    var html = "";
    // 1. Smartest within each cost band
    if (costEdges.length > 1) {
      var g1 = groupBy(costEdges, function (p) { return p.cost; }).map(function (members, i) {
        var scored = members.filter(function (p) { return p.score != null; });
        var lead = scored.length ? Math.max.apply(null, scored.map(function (p) { return p.score; })) : null;
        var choices = scored.filter(function (p) { return lead != null && p.score >= lead - scoreWin; }).sort(function (a, b) { return cmpDesc(a.score, b.score); });
        return { label: "Cost band " + (i + 1) + " · " + costLabel(i), members: members, choices: choices };
      });
      var h1 = bandGroup("Smartest within each task-cost band", "Highest scores only within each listed task-cost range; near-leaders sit within " + fmtN(scoreWin, 2) + " points of the band's best score.", g1);
      // 5. Fastest within each cost band
      var g5 = groupBy(costEdges, function (p) { return p.cost; }).map(function (members, i) {
        var timed = members.filter(function (p) { return p.time != null; });
        var lead = timed.length ? Math.min.apply(null, timed.map(function (p) { return p.time; })) : null;
        var choices = timed.filter(function (p) { return lead != null && p.time <= lead * (1 + timeWin); }).sort(function (a, b) { return cmpAsc(a.time, b.time); });
        return { label: "Cost band " + (i + 1) + " · " + costLabel(i), members: members, choices: choices };
      });
      var h5 = bandGroup("Fastest within each task-cost band", "Disjoint task-cost tiers; near-leaders are within " + Math.round(timeWin * 100) + "% of the band's fastest time.", g5, "time", "Time / task (lower is faster)");
    }
    if (sEdges.length > 1) {
      // 2. Lowest cost within each score band
      var g2 = groupBy(sEdges, function (p) { return p.score; }).map(function (members, i) {
        var priced = members.filter(function (p) { return p.cost != null; });
        var lead = priced.length ? Math.min.apply(null, priced.map(function (p) { return p.cost; })) : null;
        var choices = priced.filter(function (p) { return lead != null && p.cost <= lead * (1 + priceWin); }).sort(function (a, b) { return cmpAsc(a.cost, b.cost); });
        return { label: scoreWord() + " band " + (i + 1) + " · " + scoreLabel(i), members: members, choices: choices };
      });
      var h2 = bandGroup("Lowest task cost within each score band", "Bands split the selected score range evenly; each model appears in one score band; near-price window " + Math.round(priceWin * 100) + "%.", g2);
      // 3. Fastest within each score band
      var g3 = groupBy(sEdges, function (p) { return p.score; }).map(function (members, i) {
        var timed = members.filter(function (p) { return p.time != null; });
        var lead = timed.length ? Math.min.apply(null, timed.map(function (p) { return p.time; })) : null;
        var choices = timed.filter(function (p) { return lead != null && p.time <= lead * (1 + timeWin); }).sort(function (a, b) { return cmpAsc(a.time, b.time); });
        return { label: scoreWord() + " band " + (i + 1) + " · " + scoreLabel(i), members: members, choices: choices };
      });
      var h3 = bandGroup("Fastest within each score band", "Within " + Math.round(timeWin * 100) + "% of the band's speed leader.", g3, "time", "Time / task (lower is faster)");
    }
    if (timeEdges.length > 1) {
      // 4. Smartest within each speed band (slowest first, as Site 1)
      var g4raw = groupBy(timeEdges, function (p) { return p.time; });
      var g4 = g4raw.map(function (members, i) {
        var scored = members.filter(function (p) { return p.score != null; });
        var lead = scored.length ? Math.max.apply(null, scored.map(function (p) { return p.score; })) : null;
        var choices = scored.filter(function (p) { return lead != null && p.score >= lead - scoreWin; }).sort(function (a, b) { return cmpDesc(a.score, b.score); });
        return { label: (i === g4raw.length - 1 ? "Slowest speed band" : i === 0 ? "Fastest speed band" : "Speed band " + (i + 1)) + " · " + timeLabel(i), members: members, choices: choices };
      }).reverse();
      var h4 = bandGroup("Smartest within each speed band", "Selected speed values split into ordered, non-overlapping bands from slowest to fastest; near-leaders within the score window.", g4, "score", scoreWord() + " (higher is better)");
      // 6. Lowest price within each speed target
      var timedAll = rows.filter(function (p) { return p.time != null; }).map(function (p) { return p.time; }).sort(function (a, b) { return a - b; });
      var tiers = [];
      for (var ti = 0; ti < costEdges.length - 1; ti++) tiers.push(ti);
      var g6 = [50, 75, 90, 100].map(function (q) {
        var thr = timedAll.length ? timedAll[Math.min(timedAll.length - 1, Math.ceil((q / 100) * timedAll.length) - 1)] : null;
        var cells = tiers.map(function (ci) {
          var qual = rows.filter(function (p) { return p.cost != null && p.time != null && thr != null && p.time <= thr && bandIndex(p.cost, costEdges) === ci; });
          if (!qual.length) return { label: "Tier " + (ci + 1) + " · " + costLabel(ci), members: [], choices: [], leader: null, empty: '<span class="muted">No model meets this target in this band.</span>' };
          var cheapest = qual.slice().sort(function (a, b) { return cmpAsc(a.cost, b.cost); })[0];
          var near = qual.filter(function (p) { return p.cost <= cheapest.cost * (1 + priceWin); }).sort(function (a, b) { return cmpAsc(a.cost, b.cost); });
          return { label: "Tier " + (ci + 1) + " · " + costLabel(ci), members: qual, choices: near, leader: cheapest };
        });
        return { label: q === 100 ? "Fastest available (100th percentile)" : "At least " + q + "th percentile", threshold: thr, cells: cells };
      });
      var targetRows = g6.map(function (target) {
        return "<tr><th scope=\\\"row\\\"><strong>" + target.label + '</strong><br><span class="muted">time ≤ ' + fmtT(target.threshold) + "</span></th>" + target.cells.map(function (cell) {
          return "<td>" + (cell.choices.length ? cell.choices.map(function (p) { return chipFor(p, cell.leader); }).join(" ") : cell.empty) + "</td>";
        }).join("") + "</tr>";
      }).join("");
      var g6Lanes = [];
      g6.forEach(function (target) {
        target.cells.forEach(function (cell) {
          g6Lanes.push({ label: target.label + " · " + cell.label, members: cell.members, choices: cell.choices, leader: cell.leader });
        });
      });
      var h6 = '<h3>Lowest price within each speed target</h3><p class="muted">Each speed target is split into the selected number of log-spaced task-cost bands; nearby options within ' + Math.round(priceWin * 100) + "% of the tier's cheapest.</p>" +
        '<div class="tblwrap"><table class="xpl-table"><caption>Lowest price within each speed target</caption><thead><tr><th scope="col">Speed target</th>' + tiers.map(function (ci) { return "<th scope=\\\"col\\\">Tier " + (ci + 1) + " · " + esc(costLabel(ci)) + "</th>"; }).join("") + "</tr></thead><tbody>" + targetRows + "</tbody></table></div>";
      h6 += bandLaneChart("Lowest price within each speed target", "cost", "USD / task (lower is cheaper)", g6Lanes);
    }
    html = (typeof h1 !== "undefined" ? h1 : "") + (typeof h2 !== "undefined" ? h2 : "") + (typeof h3 !== "undefined" ? h3 : "") + (typeof h4 !== "undefined" ? h4 : "") + (typeof h5 !== "undefined" ? h5 : "") + (typeof h6 !== "undefined" ? h6 : "");
    wrap.innerHTML = html || '<p class="muted">No banded comparisons can be built — the visible configurations lack the measures they need.</p>';
    wrap.querySelectorAll("[data-pick]").forEach(function (b) {
      b.addEventListener("click", function () { selectPoint(b.getAttribute("data-pick")); });
      b.addEventListener("keydown", function (ev) {
        if (ev.key === "Enter" || ev.key === " ") { ev.preventDefault(); selectPoint(b.getAttribute("data-pick")); }
      });
    });
  }
  function renderAaTable() {
    var tb = document.getElementById("xpl-rows");
    if (!tb) return;
    var SORT_GETTERS = { perDollar: function (p) { return p.pointsPerDollar != null ? p.pointsPerDollar : p.scorePerDollar; }, perSecond: function (p) { return p.pointsPerSecond != null ? p.pointsPerSecond : p.scorePerSecond; } };
    var rows = S.sortKey ? sortRows(visPoints(), SORT_GETTERS[S.sortKey] || S.sortKey, S.sortDir) : sortPoints(visPoints());
    var ratios = S.baseline ? baselineRatios(rows, [S.baseline]) : null;
    function baseCells(p) {
      if (!ratios) return "";
      var r = ratios[p.key] && ratios[p.key].vs[S.baseline];
      return '<td class="num xpl-costcol">' + fmtN(r ? r.scorePerCost : null, 2) + "x</td>" + '<td class="num">' + fmtN(r ? r.scorePerSpeed : null, 2) + "x</td>";
    }
    var scoreHead = document.getElementById("xpl-scorehead");
    if (scoreHead) scoreHead.textContent = S.persp === "cai" ? "CAI score" : "Intelligence";
    document.getElementById("xpl-count").textContent =
      rows.length + " of " + POINTS.length + " ranked configurations · sorted by the " + S.xp + " perspective · dots are colored by model family; white-ringed dots sit on that chart's frontier, dimmed dots are beaten on its axes";
    tb.innerHTML = rows.map(function (p) {
      return '<tr data-sel="' + esc(p.key) + '" data-key="' + esc(p.key) + '"><td><strong>' + esc(p.family) + '</strong><br><span class="muted">' + esc(p.creator) + (p.qualStatus ? " · " + esc(p.qualStatus) : "") + '</span></td><td>' + esc(p.effort || "—") + '</td><td class="num">' + fmtN(p.score, 1) + '</td><td class="num xpl-costcol">' + fmt$(p.cost) + '</td><td class="num">' + fmtT(p.time) + '</td><td class="num">' + fmtT(p.ttft) + '</td><td class="num">' + fmtN(p.tps, 0) + '</td><td class="num xpl-costcol">' + fmtN(p.pointsPerDollar != null ? p.pointsPerDollar : p.scorePerDollar, 1) + '</td><td class="num">' + fmtN(p.pointsPerSecond != null ? p.pointsPerSecond : p.scorePerSecond, 3) + "</td>" + baseCells(p) + "</tr>";
    }).join("");
    tb.querySelectorAll("tr").forEach(function (tr) {
      tr.addEventListener("click", function () { selectPoint(tr.getAttribute("data-key")); });
    });
    document.querySelectorAll("[data-sortkey]").forEach(function (th) {
      var k = th.getAttribute("data-sortkey");
      th.textContent = th.textContent.replace(/ [↑↓]$/, "") + (S.sortKey === k ? (S.sortDir === "asc" ? " ↑" : " ↓") : "");
    });
    var bc = document.getElementById("xpl-bcosthead"), bs = document.getElementById("xpl-bspeedhead");
    if (bc) bc.style.display = S.baseline ? "" : "none";
    if (bs) bs.style.display = S.baseline ? "" : "none";
    markSelectedRow();
  }
  // --- task-assumptions projection (Site 2, in the Model Explorer) -------
  // ESTIMATED, NOT MEASURED: priced only from the token rates recorded
  // in the ainventory price event log (the catalog's rate bands). A
  // missing required rate yields an unknown cost, never a guess.
  // cost = turns x [uncached input + cache reads + cache writes +
  // (answer + thinking) output]; time = turns x (measured TTFT +
  // answer / measured throughput + tool overhead), borrowing the
  // family timing from either benchmark point set.
  function project(e) {
    return projectTask(e, S.pj, AA_POINTS, CAI_POINTS);
  }
  function renderModels() {
    var tb = document.getElementById("xpl-mrows");
    if (!tb) return;
    if (!CATALOG) return;
    var rows = sortCatalog(visCatalog());
    document.getElementById("xpl-mcount").textContent =
      rows.length + " of " + CATALOG.length + " catalog models · sorted by the " + S.xp + " perspective" +
      (S.xp === "api" ? " (effective $/task = AA list $/task ÷ the plan's multiplier, where the registry records one)" : "");
    tb.innerHTML = rows.map(function (e) {
      var ec = effCost(e);
      var proj = project(e);
      return '<tr data-sel="' + esc(e.id) + '" data-mid="' + esc(e.id) + '"><td><strong>' + esc(e.name) + '</strong><br><span class="muted"><code>' + esc(e.id) + "</code></span></td>" +
        "<td>" + esc(e.providerName) + (e.plan ? '<br><span class="badge src">' + esc(e.plan) + "</span>" : "") + (e.disabled ? ' <span class="badge stale">disabled</span>' : "") + '</td>' +
        '<td class="num">' + fmtCtx(e.contextLimit) + "</td>" +
        "<td>" + (e.variants.length ? esc(e.variants.join(", ")) : "—") + "</td>" +
        "<td>" + (e.routes.length ? e.routes.map(function (r) { return '<span class="badge open">' + esc(r) + "</span>"; }).join(" ") : '<span class="muted">—</span>') + "</td>" +
        '<td class="num">' + fmtN(e.aaScore, 1) + '</td><td class="num xpl-costcol">' + fmt$(e.aaCostPerTask) + '</td><td class="num xpl-costcol">' + (ec != null ? fmt$(ec) : "—") + '</td><td class="num">' + fmtT(e.aaTimePerTask) + '</td>' +
        '<td class="num xpl-costcol">' + (proj.cost != null ? "~" + fmt$(proj.cost) : "—") + '</td><td class="num">' + (proj.time != null ? "~" + fmtT(proj.time) : "—") + '</td><td class="num xpl-costcol">' + (e.aaScore != null && proj.cost ? fmtN(e.aaScore / proj.cost, 1) : "—") + "</td></tr>";
    }).join("");
    tb.querySelectorAll("tr").forEach(function (tr) {
      tr.addEventListener("click", function () { selectCatalog(tr.getAttribute("data-mid")); });
    });
    markSelectedRow();
  }
  function buildFilterOptions() {
    var creatorSel = document.getElementById("xf-creator");
    var creators = [];
    var seenC = {};
    POINTS.forEach(function (p) {
      var k = p.creator.toLowerCase();
      if (!seenC[k]) { seenC[k] = true; creators.push(p.creator); }
    });
    creators.sort();
    creatorSel.innerHTML = '<option value="">All creators</option>' + creators.map(function (c) { return '<option value="' + esc(c) + '"' + (S.creator === c ? " selected" : "") + ">" + esc(c) + "</option>"; }).join("");
    var benchSel = document.getElementById("xf-bench");
    var benches = [];
    POINTS.forEach(function (p) { if (p.benchmark && benches.indexOf(p.benchmark) < 0) benches.push(p.benchmark); });
    benchSel.innerHTML = '<option value="">All benchmarks</option>' + benches.map(function (b) { return '<option value="' + esc(b) + '"' + (S.bench === b ? " selected" : "") + ">" + esc(b) + "</option>"; }).join("");
    var effWrap = document.getElementById("xf-efforts");
    var efforts = [];
    POINTS.forEach(function (p) { var e = p.effort || ""; if (efforts.indexOf(e) < 0) efforts.push(e); });
    efforts.sort(function (a, b) { return eord(a) - eord(b); });
    effWrap.innerHTML = "";
    efforts.forEach(function (e) {
      var b = document.createElement("button");
      b.type = "button";
      b.className = "xchip" + (S.effortAny && !S.efforts[e] ? " off" : "");
      b.textContent = e || "(none)";
      b.addEventListener("click", function () {
        if (!S.effortAny) { S.effortAny = true; S.efforts = {}; S.efforts[e] = true; }
        else if (S.efforts[e]) { delete S.efforts[e]; if (!Object.keys(S.efforts).length) S.effortAny = false; }
        else S.efforts[e] = true;
        render();
      });
      effWrap.appendChild(b);
    });
    buildFamOptions();
    buildProviderOptions();
  }
  function buildFamOptions() {
    var sel = document.getElementById("xf-fam");
    var label = document.getElementById("xf-fam-label");
    var wrap = document.getElementById("xf-fam-wrap");
    if (S.tab === "models") {
      // Providers are the chip multi-select in the models filter row
      // (buildProviderOptions); the family select has no models-tab
      // role, so its wrap hides instead of doubling as a second,
      // single-select provider filter.
      if (wrap) wrap.hidden = true;
      return;
    }
    if (wrap) wrap.hidden = false;
    label.textContent = "Family";
    sel.innerHTML = '<option value="">All families</option>' + famList.slice().sort().map(function (f) { return '<option value="' + esc(f) + '"' + (S.fam === f ? " selected" : "") + ">" + esc(f) + "</option>"; }).join("");
  }
  function buildProviderOptions() {
    var wrap = document.getElementById("xf-providers");
    if (!wrap) return;
    wrap.innerHTML = "";
    if (!CATALOG) return;
    var seen = {};
    var providers = [];
    CATALOG.forEach(function (e) { if (!seen[e.provider]) { seen[e.provider] = true; providers.push([e.provider, e.providerName]); } });
    providers.sort(function (a, b) { return a[1] < b[1] ? -1 : 1; });
    providers.forEach(function (o) {
      var b = document.createElement("button");
      b.type = "button";
      b.className = "xchip" + (S.providerAny && !S.providers[o[0]] ? " off" : "");
      b.textContent = o[1];
      b.addEventListener("click", function () {
        if (!S.providerAny) { S.providerAny = true; S.providers = {}; S.providers[o[0]] = true; }
        else if (S.providers[o[0]]) { delete S.providers[o[0]]; if (!Object.keys(S.providers).length) S.providerAny = false; }
        else S.providers[o[0]] = true;
        render();
      });
      wrap.appendChild(b);
    });
  }
  function applyTab() {
    document.querySelectorAll("[data-xtab]").forEach(function (b) {
      b.classList.toggle("active", b.getAttribute("data-xtab") === S.tab);
    });
    document.getElementById("xtab-aa").hidden = S.tab !== "aa";
    document.getElementById("xtab-models").hidden = S.tab !== "models";
    document.querySelectorAll("[data-tabaa]").forEach(function (el) { el.hidden = S.tab !== "aa"; });
    document.querySelectorAll("[data-tabmodels]").forEach(function (el) { el.hidden = S.tab !== "models"; });
    buildFamOptions();
  }
  function render() {
    document.body.classList.toggle("xp-plan", S.xp === "plan");
    document.getElementById("xp-note").textContent = XP_NOTES[S.xp];
    document.querySelectorAll("[data-xp]").forEach(function (b) {
      b.classList.toggle("active", b.getAttribute("data-xp") === S.xp);
    });
    document.querySelectorAll("[data-persp]").forEach(function (b) {
      b.classList.toggle("active", b.getAttribute("data-persp") === S.persp);
    });
    var pnote = document.getElementById("xpl-persp-note");
    if (pnote) {
      pnote.textContent = S.persp === "cai"
        ? "Coding agents perspective — the Coding Agent Index observations (model + effort + agent harness): score is the CAI score, cost the published cost per coding task, time the average agent wall time per task. Kept separate from the Intelligence Index; the two are never converted."
        : "Intelligence & decode perspective — the AA Intelligence Index rankings: score is the Intelligence Index, cost and time the import's per-task figures (time excludes TTFT).";
    }
    applyTab();
    if (S.tab === "aa") { renderLegend(); renderTray(); drawCharts(); renderUnpriced(); renderAaTable(); renderPareto(); renderRecs(); renderBandComparisons(); }
    else renderModels();
    enhanceCharts();
  }
  document.querySelectorAll("[data-xtab]").forEach(function (b) {
    b.addEventListener("click", function () {
      S.tab = b.getAttribute("data-xtab");
      S.fam = "";
      history.replaceState(null, "", S.tab === "models" ? "#models" : "#aa");
      render();
    });
  });
  document.querySelectorAll("[data-xp]").forEach(function (b) {
    b.addEventListener("click", function () { S.xp = b.getAttribute("data-xp"); render(); });
  });
  function setPerspective(persp) {
    if (S.persp === persp) return;
    S.persp = persp;
    POINTS = persp === "cai" ? CAI_POINTS : AA_POINTS;
    S.creator = ""; S.fam = ""; S.bench = ""; S.effortAny = false; S.efforts = {}; S.providerAny = false; S.providers = {};
    S.off = {}; S.hidden = []; S.sel = null; S.baseline = "";
    showDetail(null);
    buildFilterOptions();
    buildBaselineOptions();
    render();
  }
  document.querySelectorAll("[data-persp]").forEach(function (b) {
    b.addEventListener("click", function () { setPerspective(b.getAttribute("data-persp")); });
  });
  var pfCutEl = document.getElementById("xf-pfcut");
  if (pfCutEl) pfCutEl.addEventListener("change", function (ev) { S.pfCut = ev.target.value; renderPareto(); });
  var bandsEl = document.getElementById("xf-bands");
  if (bandsEl) bandsEl.addEventListener("change", function (ev) { S.bandCount = Number(ev.target.value); renderRecs(); renderBandComparisons(); enhanceCharts(); });
  var bandScaleEl = document.getElementById("xf-bandscale");
  if (bandScaleEl) bandScaleEl.addEventListener("change", function (ev) { S.bandScale = Number(ev.target.value); renderBandComparisons(); enhanceCharts(); });
  function syncRecInputs() {
    document.querySelectorAll("[data-rec]").forEach(function (el) {
      var f = el.getAttribute("data-rec");
      if (S.rec[f] != null) el.value = S.rec[f];
    });
    var sp = document.getElementById("rec-speedpriority-val");
    if (sp) sp.textContent = S.rec.speedPriority + "% speed";
  }
  document.querySelectorAll("[data-rec]").forEach(function (el) {
    el.addEventListener("input", function () {
      S.rec[el.getAttribute("data-rec")] = el.value;
      syncRecInputs();
      renderRecs();
    });
    el.addEventListener("change", function () {
      S.rec[el.getAttribute("data-rec")] = el.value;
      renderRecs();
    });
  });
  var REC_PRESETS = {
    fast: { allow: 5, speedPriority: 95, priceAllow: 200, slowAllow: 1.5, altWindow: 2, diversity: "lab", dominance: "lab", perBand: 3 },
    balanced: { allow: 5, speedPriority: 50, priceAllow: 500, slowAllow: 2, altWindow: 2, diversity: "lab", dominance: "lab", perBand: 3 },
    diversity: { allow: 7, speedPriority: 60, priceAllow: 500, slowAllow: 2.5, altWindow: 3, diversity: "labsfirst", dominance: "lab", perBand: 4 },
    budget: { allow: 5, speedPriority: 15, priceAllow: 100, slowAllow: 3, altWindow: 2, diversity: "lab", dominance: "all", perBand: 3 },
    reset: { allow: 5, speedPriority: 85, priceAllow: 500, slowAllow: 2, altWindow: 2, diversity: "lab", dominance: "lab", perBand: 3, maxCost: "", maxSec: "" },
  };
  document.querySelectorAll("[data-recpreset]").forEach(function (b) {
    b.addEventListener("click", function () {
      var preset = REC_PRESETS[b.getAttribute("data-recpreset")];
      if (!preset) return;
      Object.keys(preset).forEach(function (k) { S.rec[k] = preset[k]; });
      syncRecInputs();
      renderRecs();
    });
  });
  function syncPjInputs() {
    document.querySelectorAll("[data-pj]").forEach(function (el) {
      var f = el.getAttribute("data-pj");
      if (S.pj[f] != null) el.value = S.pj[f];
    });
    var cv = document.getElementById("pj-cacheread-val");
    if (cv) cv.textContent = S.pj.cacheReadPct + "%";
  }
  document.querySelectorAll("[data-pj]").forEach(function (el) {
    var on = function () { S.pj[el.getAttribute("data-pj")] = el.value; syncPjInputs(); if (S.tab === "models") renderModels(); };
    el.addEventListener("input", on);
    el.addEventListener("change", on);
  });
  var PJ_PRESETS = {
    small: { turns: 2, inputTokens: 8000, answerTokens: 1200, thinkingTokens: 2000, cacheReadPct: 60, writesPct: 0, overheadSec: 8 },
    feature: { turns: 4, inputTokens: 25000, answerTokens: 3000, thinkingTokens: 6000, cacheReadPct: 50, writesPct: 5, overheadSec: 10 },
    repo: { turns: 8, inputTokens: 60000, answerTokens: 4000, thinkingTokens: 8000, cacheReadPct: 70, writesPct: 5, overheadSec: 15 },
  };
  document.querySelectorAll("[data-pjpreset]").forEach(function (b) {
    b.addEventListener("click", function () {
      var preset = PJ_PRESETS[b.getAttribute("data-pjpreset")];
      if (!preset) return;
      Object.keys(preset).forEach(function (k) { S.pj[k] = preset[k]; });
      syncPjInputs();
      if (S.tab === "models") renderModels();
    });
  });
  var recExport = document.getElementById("xpl-rec-export");
  if (recExport) {
    recExport.addEventListener("click", function () {
      var out = computeRecs();
      var lines = ["band,rank,model,effort,lab,harness,score,cost_per_task,time_per_task_s,tradeoff_multiple"];
      out.bands.forEach(function (band) {
        var bestFactor = band.picks.length ? band.picks[0].factor : 1;
        band.picks.forEach(function (r, ri) {
          lines.push([band.label, ri + 1, r.p.family, r.p.effort || "", r.p.creator, r.p.harness || "", r.p.score, r.p.cost, r.p.time, (r.factor / bestFactor).toFixed(2)].map(function (v) { return '"' + String(v == null ? "" : v).replace(/"/g, '""') + '"'; }).join(","));
        });
      });
      downloadFile("cost-explorer-shortlist.csv", lines.join("\\n"), "text/csv");
    });
  }
  document.getElementById("xf-q").addEventListener("input", function (ev) { S.q = ev.target.value.trim().toLowerCase(); render(); });
  document.getElementById("xf-creator").addEventListener("change", function (ev) { S.creator = ev.target.value; render(); });
  document.getElementById("xf-fam").addEventListener("change", function (ev) { S.fam = ev.target.value; render(); });
  document.getElementById("xf-bench").addEventListener("change", function (ev) { S.bench = ev.target.value; render(); });
  document.getElementById("xf-elig").addEventListener("change", function (ev) { S.elig = ev.target.checked; render(); });
  document.getElementById("xf-pooled").addEventListener("change", function (ev) { S.pooledOnly = ev.target.checked; render(); });
  document.getElementById("xf-measured").addEventListener("change", function (ev) { S.measuredOnly = ev.target.checked; render(); });
  document.getElementById("xf-nodis").addEventListener("change", function (ev) { S.hideDisabled = ev.target.checked; render(); });
  document.querySelectorAll("[data-sizeby]").forEach(function (cb) {
    cb.addEventListener("change", function () { S.sizeBy[cb.getAttribute("data-sizeby")] = cb.checked; drawCharts(); });
  });
  document.querySelectorAll("[data-overlay]").forEach(function (cb) {
    cb.addEventListener("change", function () { S.overlays[cb.getAttribute("data-overlay")] = cb.checked; drawCharts(); });
  });
  document.querySelectorAll("[data-dl]").forEach(function (b) {
    b.addEventListener("click", function () { downloadPng(b.getAttribute("data-dl")); });
  });
  document.querySelectorAll("[data-tmode]").forEach(function (b) {
    b.addEventListener("click", function () { S.tasksMode = b.getAttribute("data-tmode"); drawCharts(); });
  });
  var baselineSel = document.getElementById("xf-baseline");
  if (baselineSel) baselineSel.addEventListener("change", function (ev) { S.baseline = ev.target.value; drawCharts(); renderAaTable(); });
  document.querySelectorAll("[data-sortkey]").forEach(function (th) {
    th.style.cursor = "pointer";
    th.title = "Sort by this column";
    th.addEventListener("click", function () {
      var k = th.getAttribute("data-sortkey");
      if (S.sortKey === k) S.sortDir = S.sortDir === "asc" ? "desc" : "asc";
      else { S.sortKey = k; S.sortDir = th.getAttribute("data-dir") || "desc"; }
      renderAaTable();
    });
  });
  document.querySelectorAll("[data-dom]").forEach(function (el) {
    el.addEventListener("change", function () { S.dom[el.getAttribute("data-dom")] = el.checked; render(); });
  });
  // --- inequality filter rows ---------------------------------------------
  var INEQ_METRICS = [["score", "Score"], ["cost", "Task cost"], ["time", "Selected speed (time / task)"], ["tps", "Output speed (tok/s)"], ["gapScore", "Pareto gap: score gap (vs cost frontier)"], ["gapCost", "Pareto gap: price premium (vs score)"], ["gapSpeed", "Pareto gap: speed gap (vs time frontier)"]];
  function renderIneqConds() {
    var wrap = document.getElementById("xf-ineq-conds");
    if (!wrap) return;
    wrap.innerHTML = S.ineq.conds.map(function (c, i) {
      var metricOpts = INEQ_METRICS.map(function (m) { return '<option value="' + m[0] + '"' + (c.metric === m[0] ? " selected" : "") + ">" + m[1] + "</option>"; }).join("");
      var opOpts = ["<", "<=", "=", ">=", ">"].map(function (o) { return '<option value="' + o + '"' + (c.op === o ? " selected" : "") + ">" + o + "</option>"; }).join("");
      return '<span class="xpl-ineq-row"><label>Condition ' + (i + 1) + ' <select data-ineq-metric="' + i + '">' + metricOpts + '</select></label> <select data-ineq-op="' + i + '" aria-label="Operator">' + opOpts + '</select> <input type="number" step="any" data-ineq-value="' + i + '" value="' + esc(c.value) + '" style="width:7em"> <button type="button" class="xpl-ctl-btn" data-ineq-remove="' + i + '">Remove</button></span>';
    }).join("");
    wrap.querySelectorAll("[data-ineq-metric]").forEach(function (el) { el.addEventListener("change", function () { S.ineq.conds[Number(el.getAttribute("data-ineq-metric"))].metric = el.value; render(); }); });
    wrap.querySelectorAll("[data-ineq-op]").forEach(function (el) { el.addEventListener("change", function () { S.ineq.conds[Number(el.getAttribute("data-ineq-op"))].op = el.value; render(); }); });
    wrap.querySelectorAll("[data-ineq-value]").forEach(function (el) { el.addEventListener("input", function () { S.ineq.conds[Number(el.getAttribute("data-ineq-value"))].value = el.value; render(); }); });
    wrap.querySelectorAll("[data-ineq-remove]").forEach(function (el) { el.addEventListener("click", function () { S.ineq.conds.splice(Number(el.getAttribute("data-ineq-remove")), 1); renderIneqConds(); render(); }); });
  }
  var ineqModeEl = document.getElementById("xf-ineq-mode");
  if (ineqModeEl) ineqModeEl.addEventListener("change", function (ev) { S.ineq.mode = ev.target.value; render(); });
  var ineqCombineEl = document.getElementById("xf-ineq-combine");
  if (ineqCombineEl) ineqCombineEl.addEventListener("change", function (ev) { S.ineq.combine = ev.target.value; render(); });
  var ineqAddEl = document.getElementById("xf-ineq-add");
  if (ineqAddEl) ineqAddEl.addEventListener("click", function () { S.ineq.conds.push({ metric: "score", op: ">=", value: "" }); renderIneqConds(); });
  // --- saved setups + downloads ---------------------------------------------
  function captureSetup() {
    return JSON.parse(JSON.stringify({
      tab: S.tab, persp: S.persp, xp: S.xp, q: S.q, creator: S.creator, fam: S.fam, bench: S.bench,
      effortAny: S.effortAny, efforts: S.efforts, providerAny: S.providerAny, providers: S.providers, off: S.off, elig: S.elig, pooledOnly: S.pooledOnly,
      measuredOnly: S.measuredOnly, hideDisabled: S.hideDisabled, hidden: S.hidden, sizeBy: S.sizeBy, overlays: S.overlays,
      tasksMode: S.tasksMode, baseline: S.baseline, sortKey: S.sortKey, sortDir: S.sortDir, dom: S.dom, pfCut: S.pfCut, bandCount: S.bandCount,
      bandScale: S.bandScale, rec: S.rec, pj: S.pj, ineq: S.ineq,
    }));
  }
  function syncStaticControls() {
    var q = document.getElementById("xf-q"); if (q) q.value = S.q;
    var elig = document.getElementById("xf-elig"); if (elig) elig.checked = S.elig;
    var pooled = document.getElementById("xf-pooled"); if (pooled) pooled.checked = S.pooledOnly;
    var meas = document.getElementById("xf-measured"); if (meas) meas.checked = S.measuredOnly;
    var nodis = document.getElementById("xf-nodis"); if (nodis) nodis.checked = S.hideDisabled;
    var pf = document.getElementById("xf-pfcut"); if (pf) pf.value = S.pfCut;
    var bd = document.getElementById("xf-bands"); if (bd) bd.value = String(S.bandCount);
    var bs = document.getElementById("xf-bandscale"); if (bs) bs.value = String(S.bandScale);
    if (ineqModeEl) ineqModeEl.value = S.ineq.mode;
    document.querySelectorAll("[data-dom]").forEach(function (el) { el.checked = !!S.dom[el.getAttribute("data-dom")]; });
    if (ineqCombineEl) ineqCombineEl.value = S.ineq.combine;
    document.querySelectorAll("[data-tmode]").forEach(function (b) { b.classList.toggle("active", b.getAttribute("data-tmode") === S.tasksMode); });
    document.querySelectorAll("[data-sizeby]").forEach(function (cb) { cb.checked = !!S.sizeBy[cb.getAttribute("data-sizeby")]; });
    document.querySelectorAll("[data-overlay]").forEach(function (cb) { cb.checked = !!S.overlays[cb.getAttribute("data-overlay")]; });
  }
  function applySetup(st) {
    if (!st || typeof st !== "object") return;
    ["tab", "persp", "xp", "q", "creator", "fam", "bench", "effortAny", "providerAny", "elig", "pooledOnly", "measuredOnly", "hideDisabled", "tasksMode", "baseline", "pfCut", "bandCount", "bandScale", "sortKey", "sortDir"].forEach(function (k) { if (st[k] !== undefined) S[k] = st[k]; });
    ["efforts", "providers", "off", "hidden", "sizeBy", "overlays", "rec", "pj", "ineq", "dom"].forEach(function (k) { if (st[k] !== undefined) S[k] = st[k]; });
    POINTS = S.persp === "cai" ? CAI_POINTS : AA_POINTS;
    S.sel = null;
    showDetail(null);
    buildFilterOptions();
    buildBaselineOptions();
    syncRecInputs();
    syncPjInputs();
    renderIneqConds();
    syncStaticControls();
    render();
  }
  function loadSetups() {
    // #2851: the store is null-prototype — on a plain object a setup
    // named "__proto__" assigns the prototype instead of becoming an
    // entry (silently never saved), and an imported "__proto__" entry
    // mutates the live store's prototype. Copy the parsed own keys
    // into Object.create(null) so every name is an ordinary entry.
    var out = Object.create(null);
    try {
      var parsed = JSON.parse(localStorage.getItem("xpl-setups-v1") || "{}");
      if (parsed && typeof parsed === "object") Object.keys(parsed).forEach(function (k) { out[k] = parsed[k]; });
    } catch (e) { /* unreadable storage: empty store */ }
    return out;
  }
  function storeSetups(obj) {
    try { localStorage.setItem("xpl-setups-v1", JSON.stringify(obj)); } catch (e) { /* storage unavailable: setups stay session-only */ }
  }
  function refreshSetupList(selected) {
    var sel = document.getElementById("xf-setup-list");
    if (!sel) return;
    var names = Object.keys(loadSetups()).sort();
    sel.innerHTML = names.length ? names.map(function (n) { return '<option value="' + esc(n) + '"' + (n === selected ? " selected" : "") + ">" + esc(n) + "</option>"; }).join("") : '<option value="">(none saved)</option>';
  }
  var setupSaveEl = document.getElementById("xf-setup-save");
  if (setupSaveEl) setupSaveEl.addEventListener("click", function () {
    var nameEl = document.getElementById("xf-setup-name");
    var name = nameEl && nameEl.value.trim() ? nameEl.value.trim() : "Setup " + (Object.keys(loadSetups()).length + 1);
    var all = loadSetups();
    all[name] = captureSetup();
    storeSetups(all);
    refreshSetupList(name);
  });
  var setupLoadEl = document.getElementById("xf-setup-load");
  if (setupLoadEl) setupLoadEl.addEventListener("click", function () {
    var sel = document.getElementById("xf-setup-list");
    if (sel && sel.value) applySetup(loadSetups()[sel.value]);
  });
  var setupExportEl = document.getElementById("xf-setup-export");
  if (setupExportEl) setupExportEl.addEventListener("click", function () { downloadFile("cost-explorer-setups.json", JSON.stringify(loadSetups(), null, 2), "application/json"); });
  var setupImportEl = document.getElementById("xf-setup-import");
  if (setupImportEl) setupImportEl.addEventListener("change", function () {
    var file = setupImportEl.files && setupImportEl.files[0];
    if (!file) return;
    var reader = new FileReader();
    reader.onload = function () {
      try {
        var parsed = JSON.parse(String(reader.result));
        if (parsed && typeof parsed === "object") {
          var all = loadSetups();
          Object.keys(parsed).forEach(function (k) { all[k] = parsed[k]; });
          storeSetups(all);
          refreshSetupList();
        }
      } catch (e) { /* unreadable file: nothing imported */ }
    };
    reader.readAsText(file);
  });
  var setupResetEl = document.getElementById("xf-setup-reset");
  if (setupResetEl) setupResetEl.addEventListener("click", function () {
    applySetup({
      tab: "aa", persp: "aa", xp: "api", q: "", creator: "", fam: "", bench: "", effortAny: false, efforts: {}, off: {},
      elig: false, pooledOnly: false, measuredOnly: false, hideDisabled: false, hidden: [], sizeBy: {}, tasksMode: "per",
      baseline: "", pfCut: "50", bandCount: 3, bandScale: 1,
      rec: { allow: 5, speedPriority: 85, priceAllow: 500, slowAllow: 2, altWindow: 2, diversity: "lab", dominance: "lab", perBand: 3, maxCost: "", maxSec: "" },
      pj: { turns: 3, inputTokens: 20000, answerTokens: 2000, thinkingTokens: 4000, cacheReadPct: 50, writesPct: 0, overheadSec: 10, window: "peak" },
      ineq: { mode: "off", combine: "and", conds: [{ metric: "score", op: ">=", value: "" }, { metric: "cost", op: "<=", value: "" }, { metric: "time", op: "<=", value: "" }] },
      sortKey: "", sortDir: "desc", dom: { dominated: false, slower: false, dumber: false, pricier: false },
    });
  });
  function csvCell(v) { return '"' + String(v == null ? "" : v).replace(/"/g, '""') + '"'; }
  var dlJsonEl = document.getElementById("xf-dl-json");
  if (dlJsonEl) dlJsonEl.addEventListener("click", function () { downloadFile("cost-explorer-data-" + String(DATA.batch || "latest").slice(0, 8) + ".json", JSON.stringify(DATA, null, 2), "application/json"); });
  var dlCsvEl = document.getElementById("xf-dl-csv");
  if (dlCsvEl) dlCsvEl.addEventListener("click", function () {
    var lines = [];
    if (S.tab === "models") {
      lines.push(["model_id", "name", "provider", "plan", "context", "aa_score", "aa_cost_per_task", "proj_cost_per_task", "proj_time_per_task_s"].join(","));
      sortCatalog(visCatalog()).forEach(function (e) {
        var proj = project(e);
        lines.push([e.id, e.name, e.providerName, e.plan || "", e.contextLimit, e.aaScore, e.aaCostPerTask, proj.cost, proj.time].map(csvCell).join(","));
      });
      downloadFile("cost-explorer-models-visible.csv", lines.join("\\n"), "text/csv");
    } else {
      lines.push(["family", "effort", "creator", "benchmark", "harness", "score", "cost_per_task", "time_per_task_s", "ttft_s", "tokens_per_second", "tokens_per_task"].join(","));
      sortPoints(visPoints()).forEach(function (p) {
        lines.push([p.family, p.effort || "", p.creator, p.benchmark || "", p.harness || "", p.score, p.cost, p.time, p.ttft, p.tps, p.tokensPerTask].map(csvCell).join(","));
      });
      downloadFile("cost-explorer-configurations-visible.csv", lines.join("\\n"), "text/csv");
    }
  });
  renderIneqConds();
  refreshSetupList();
  buildFilterOptions();
  buildBaselineOptions();
  render();
})();
</script>`;

function fmtCost(v: number | null): string {
  return v == null ? "—" : `$${Number(v.toFixed(4))}`;
}

function fmtSec(v: number | null): string {
  if (v == null) return "—";
  if (v < 60) return `${Number(v.toFixed(1))}s`;
  return `${Math.floor(v / 60)}m ${Math.round(v % 60)}s`;
}

function fmtTimed(v: number | null): string {
  return v == null ? '<span class="muted">not measured</span>' : fmtSec(v);
}

/** "openai|gpt-6-luna|max|reasoning|fallback-na" -> "gpt-6-luna · max". */
function variantLabel(modelVariantKey: string): string {
  const parts = modelVariantKey.split("|");
  return parts.length >= 3 ? `${parts[1]} · ${parts[2]}` : modelVariantKey;
}

function aaTimingsTable(testRuns: AaTestRun[]): string {
  const rows = testRuns
    .map(
      (r) =>
        `<tr><td><strong>${esc(variantLabel(r.modelVariantKey))}</strong><br><span class="muted">${esc(r.snapshotDate ?? "no snapshot date")}</span></td><td>${r.intelligence ?? "—"}</td><td>${fmtCost(r.costPerTaskUsd)}</td><td>${fmtTimed(r.ttftSeconds)}</td><td>${fmtTimed(r.firstAnswerSeconds)}</td><td>${fmtTimed(r.e2e500Seconds)}</td><td>${fmtTimed(r.timePerTaskSeconds)}</td><td>${r.outputSpeedTps ?? "—"}</td></tr>`,
    )
    .join("");
  return `<div class="tblwrap"><table class="table table-zebra"><caption>Model configurations — intelligence and cost</caption><tr><th scope="col">Configuration</th><th scope="col">Intelligence</th><th scope="col">$/task</th><th scope="col">TTFT</th><th scope="col">First answer</th><th scope="col">E2E (500 tok)</th><th scope="col">Time / task</th><th scope="col">Speed (t/s)</th></tr>${rows || '<tr><td colspan="8" class="muted">none</td></tr>'}</table></div>`;
}

function aaConfigsTable(configs: AaConfig[]): string {
  const rows = configs
    .map(
      (c) =>
        `<tr><td><strong>${esc(c.modelName)}</strong></td><td>${esc(c.effort ?? "—")}</td><td>${c.intelligence ?? "—"}</td><td>${fmtCost(c.costPerTaskUsd)}</td><td>${c.speedTps ?? "—"}</td></tr>`,
    )
    .join("");
  return `<div class="tblwrap"><table class="table table-zebra"><caption>Models — effort, intelligence, cost, and speed</caption><tr><th scope="col">Model</th><th scope="col">Effort (AA label)</th><th scope="col">Intelligence</th><th scope="col">$/task</th><th scope="col">Speed (t/s)</th></tr>${rows || '<tr><td colspan="5" class="muted">none</td></tr>'}</table></div>`;
}

function aaCaiTable(rowsIn: AaCaiRow[]): string {
  const pct = (v: number | null): string => (v == null ? "—" : `${Number((v * 100).toFixed(1))}%`);
  const rows = rowsIn
    .map(
      (r) =>
        `<tr><td><strong>${esc(r.family)}</strong><br><span class="muted">${esc(r.creator)}${r.snapshotDate ? ` · ${esc(r.snapshotDate)}` : ""}</span></td><td>${esc(r.effort ?? "—")}</td><td>${esc(r.harness ?? "—")}</td><td>${esc(r.version ?? "—")}</td><td>${r.score ?? "—"}</td><td>${pct(r.deepSwe)}</td><td>${pct(r.terminalBench)}</td><td>${pct(r.sweAtlas)}</td><td>${fmtCost(r.cost)}</td><td>${fmtTimed(r.wallSec)}</td><td>${r.tokensPerTask != null ? Math.round(r.tokensPerTask).toLocaleString("en-US") : "—"}</td></tr>`,
    )
    .join("");
  return `<h2>Coding Agent Index observations</h2>
<p class="muted">The import's Coding Agent Index rows — a model plus its agent harness and settings, checked as published. Kept strictly separate from the AA Intelligence Index rows above: no score is converted between the indexes. "Not published" costs stay blank rather than estimated. These rows power the explorer's Coding agents perspective.</p>
<div class="tblwrap"><table class="table table-zebra"><caption>Model benchmark scores</caption><tr><th scope="col">Model</th><th scope="col">Effort</th><th scope="col">Harness</th><th scope="col">Version</th><th scope="col">CAI score</th><th scope="col">DeepSWE v1.1</th><th scope="col">Terminal-Bench 4.0</th><th scope="col">SWE-Atlas-QnA</th><th scope="col">$/task</th><th scope="col">Agent wall time / task</th><th scope="col">Tokens / task</th></tr>${rows || '<tr><td colspan="11" class="muted">none</td></tr>'}</table></div>`;
}

function aaNameList(names: string[]): string {
  return names.length
    ? `<div class="strip">${names.map((n) => `<span class="badge src">${esc(n)}</span>`).join("")}</div>`
    : '<p class="muted">none</p>';
}

/** JSON for a <script type="application/json"> block: `<` is the only
 * character that can break out of it, so it crosses as its escape. */
function jsonForScript(json: string): string {
  return json.replace(/</g, "\\u003c");
}

const AA_COLLECTOR_CADENCE_MS = 5 * 60_000;

function aaFreshnessChips(detail: ProvenanceDetail | undefined, section: AaFeedSection | null): string {
  if (!detail) return "";
  const aaFacet = detail.aa;
  const olderThanOneCycle =
    aaFacet.state === "live" && aaFacet.ageMs !== undefined && aaFacet.ageMs > AA_COLLECTOR_CADENCE_MS;
  const retainedAfterFailure = section?.status === "ok" && section.stale === true;
  const chipDetail: ProvenanceDetail =
    olderThanOneCycle || retainedAfterFailure ? { ...detail, aa: { ...aaFacet, state: "stale" } } : detail;
  return freshnessChips(chipDetail, ["aa"]);
}

export function renderAa(section: AaFeedSection | null, provenance: Provenance, detail?: ProvenanceDetail): string {
  if (!section) {
    return layout(
      "Cost Explorer",
      `<h1>Cost Explorer</h1><div class="banner">The AA data has not reached the dashboard yet — the fleet-feed collector reads it from the Turso fleet-platform store (ainventory), and no AA section is in the current feed. The data of record is the ainventory import; this page only renders it.</div>`,
      provenance,
      "/aa",
      "",
      aaFreshnessChips(detail, null),
    );
  }
  if (section.status === "absent") {
    return layout(
      "Cost Explorer",
      `<h1>Cost Explorer</h1><div class="banner">The collector could not build the AA section on its last run (${fmtDate(section.generatedAt)}): ${esc(section.reason)}. No successful collection exists yet to fall back to — the page fills in on the first successful collection.</div>`,
      provenance,
      "/aa",
      "",
      aaFreshnessChips(detail, section),
    );
  }
  const s: AaSection = section;
  const summary = summarizeAa(s);
  const sourcesHtml = s.sources
    .map(
      (src) =>
        `<li><code>${esc(src.name)}</code> (${esc(src.kind)}) — imported ${src.importedAt ? fmtDate(src.importedAt) : "—"} · file modified ${src.driveModifiedAt ? fmtDate(src.driveModifiedAt) : "—"}</li>`,
    )
    .join("");
  const cov = s.coverage;
  const coverageHtml = cov
    ? `<h2>Coverage — measured but not pooled</h2>
<p class="muted">The collector compares the AA-measured model names against the repo's <code>.opencode/opencode.jsonc</code>: the provider catalogs and every route pool's leaf targets (nested routes expanded). Matching is best-effort — normalization plus a small alias map in <code>src/aa.ts</code> — so a miss means "not matched", not "does not exist".</p>
<h3>In a catalog, in no pool (${cov.catalogNotPooledMeasured.length})</h3>
<p class="muted">Measured and reachable today, but no route sends them work — the actionable list.</p>${aaNameList(cov.catalogNotPooledMeasured)}
<h3>Measured, in no catalog (${cov.measuredNotInCatalog.length})</h3>${aaNameList(cov.measuredNotInCatalog)}
<details><summary>Catalog models with no AA measurement and no pool seat (${cov.catalogNotPooledUnmeasured.length})</summary>
<p class="muted">Catalog ids verbatim from the config — this is where unmeasured newcomers (e.g. <code>commandcode/reflection/beam:free</code>, the Reflection Beam seat) sit until the import measures them.</p>${aaNameList(cov.catalogNotPooledUnmeasured)}</details>`
    : `<h2>Coverage — measured but not pooled</h2><p class="muted">Coverage was not computed for this section — the collector could not read <code>.opencode/opencode.jsonc</code> on that run.</p>`;
  const catalog = s.catalog;
  const feedAge = detail?.aa.ageMs === undefined ? "unavailable" : ageMsText(detail.aa.ageMs);
  const aaTabAge = detail ? `<p class="muted" data-aa-age>AA data age: ${esc(feedAge)} (fleet-feed age).</p>` : "";
  const catalogTabAge = detail
    ? catalog
      ? `<p class="muted" data-catalog-age>Model catalog age: ${esc(feedAge)} (fleet-feed age; catalog and AA are read in the same collector run; no separate catalog timestamp is carried).</p>`
      : `<p class="muted" data-catalog-age>Model catalog age: unavailable — no catalog in this feed (fleet-feed age: ${esc(feedAge)}).</p>`
    : "";
  const modelCards = catalog
    ? `<div class="board">
<div class="card"><strong>${catalog.length} models</strong> in the fleet's provider catalogs<br><span class="muted">from <code>.opencode/opencode.jsonc</code>, read by the same collector run as the AA data</span></div>
<div class="card"><strong>${new Set(catalog.map((e) => e.provider)).size} providers</strong> · ${catalog.filter((e) => e.disabled).length} models behind disabled providers<br><span class="muted">Disabled means catalogued but not callable — the config disables the door, the proxy layer never does</span></div>
<div class="card"><strong>${catalog.filter((e) => e.routes.length > 0).length} pooled</strong> · ${catalog.filter((e) => e.aaScore !== null).length} AA-measured<br><span class="muted">Pooled = at least one route sends it work; measured = the import has an intelligence score for its name</span></div>
</div>`
    : `<div class="banner">The collector has not carried the fleet catalog yet — it is built from the same <code>.opencode/opencode.jsonc</code> read as the coverage section and appears once a collector build that knows it has run. No catalog is shown rather than a stale one.</div>`;
  const explorerJson = jsonForScript(
    JSON.stringify({
      batch: s.batch,
      points: aaPoints(s),
      caiPoints: s.cai ? aaCaiPoints(s.cai) : [],
      catalog: catalog ?? null,
    }),
  );
  // #3302: dataset-version selector. The selected batch scopes the whole
  // explorer (charts, tables, recommendations) because the server hands
  // renderAa that batch's section; downloads read the same embedded DATA.
  const batches = s.batches ?? [];
  const latestBatch = batches.find((b) => b.latest)?.importBatch ?? batches[0]?.importBatch ?? s.batch;
  const selectedIsLatest = s.batch === latestBatch;
  const batchOptions = batches
    .map(
      (b) =>
        `<option value="${esc(b.importBatch)}"${b.importBatch === s.batch ? " selected" : ""}>${esc(b.label)}${b.importBatch === latestBatch ? " (latest)" : ""}${b.cached ? "" : " — not cached yet"}</option>`,
    )
    .join("");
  const batchSelector =
    batches.length > 1
      ? `<div class="card" id="aa-batch-picker"><label><strong>Dataset version</strong>
<select id="xf-batch" onchange="location.href = this.value ? '/aa?batch=' + encodeURIComponent(this.value) : '/aa'">${batchOptions}</select></label>
<span class="muted">Older versions show that import's rankings, thresholds, timings and coding-agent rows. The latest is the default. Token prices remain the current <code>v_current_prices</code> overlay, not a historical price snapshot.</span></div>`
      : "";
  const body = `<h1>Cost Explorer</h1>
${s.stale ? `<div class="banner">The latest collection failed (${esc(s.staleReason ?? "unknown error")}) — showing the last good data, collected ${fmtDate(s.generatedAt)}. It refreshes automatically on the next successful collection.</div>` : ""}
${batchSelector}
<p class="muted">${s.rankings.length} ranked configurations · ${s.configs.length} AA Intelligence Index configurations · ${s.testRuns.length} timing rows${s.cai ? ` · ${s.cai.length} Coding Agent Index observations` : ""}${catalog ? ` · ${catalog.length} catalog models` : ""} · collected ${fmtDate(s.generatedAt)} from ainventory batch <code>${esc(s.batch.slice(0, 8))}…</code>${selectedIsLatest ? " (latest)" : " (historical version)"}. Machine-first: <a href="/api/aa.json${selectedIsLatest ? "" : `?batch=${encodeURIComponent(s.batch)}`}">/api/aa.json${selectedIsLatest ? "" : `?batch=${encodeURIComponent(s.batch)}`}</a>.</p>
<div class="card"><strong>Provenance</strong><br>
<span class="muted">These are the import snapshot's figures, not a live Artificial Analysis pull.${summary.snapshotVintage ? ` Test_Runs snapshots in this batch: ${esc(summary.snapshotVintage.min)} – ${esc(summary.snapshotVintage.max)}.` : ""} A new import (coding-plan-watch) flips the batch; the feed collector picks it up within minutes. The explorer renders from the feed alone — when the data pipeline lands new models (Terra, Beam), they appear here with no page change. Token prices come from <code>v_current_prices</code>, a current-price overlay that is not scoped to the selected dataset version.</span>
<ul>${sourcesHtml || '<li class="muted">source files not recorded for this batch</li>'}</ul></div>
<div class="xtabs" role="tablist"><button type="button" class="xtab" data-xtab="aa">AA Explorer</button><button type="button" class="xtab" data-xtab="models">Model Explorer</button></div>
<div class="card xpl-filters" id="xpl-filters">
<input type="search" id="xf-q" placeholder="Search models, families, providers…" aria-label="Search">
<label data-tabaa>Creator <select id="xf-creator"></select></label>
<label id="xf-fam-wrap"><span id="xf-fam-label">Family</span> <select id="xf-fam"></select></label>
<label data-tabaa>Benchmark <select id="xf-bench"></select></label>
<span class="xp-seg" role="group" aria-label="Efficiency perspective"><button type="button" data-xp="plan">Plan</button><button type="button" data-xp="api">API</button><button type="button" data-xp="pertask">Per-task</button></span>
<span class="xp-seg" role="group" aria-label="Benchmark perspective" data-tabaa><button type="button" data-persp="aa">Intelligence &amp; decode</button><button type="button" data-persp="cai">Coding agents</button></span>
<p class="muted" id="xpl-persp-note" data-tabaa style="flex-basis:100%;margin:2px 0 0"></p>
<div data-tabaa style="flex-basis:100%"><span class="muted">Efforts:</span> <span id="xf-efforts"></span> <label style="margin-left:10px"><input type="checkbox" id="xf-elig"> Eligible only</label></div>
<div data-tabmodels hidden style="flex-basis:100%"><span class="muted">Providers:</span> <span id="xf-providers"></span> <label style="margin-left:10px"><input type="checkbox" id="xf-pooled"> Pooled only</label> <label><input type="checkbox" id="xf-measured"> AA-measured only</label> <label><input type="checkbox" id="xf-nodis"> Hide disabled providers</label></div>
<div data-tabaa style="flex-basis:100%"><details><summary>Model selection and advanced filters — inequality conditions</summary>
<label>Filter mode <select id="xf-ineq-mode"><option value="off" selected>Off</option><option value="keep">Keep matching models</option><option value="reject">Keep only rejected models</option></select></label>
<label>Combine conditions <select id="xf-ineq-combine"><option value="and" selected>Match all (AND)</option><option value="or">Match any (OR)</option></select></label>
<span class="muted">Pareto-gap metrics compare against the observed frontier of the current scope: score gap (score vs cost), price premium (cost vs score), speed gap (score vs time).</span>
<div id="xf-ineq-conds"></div>
<button type="button" class="xpl-ctl-btn" id="xf-ineq-add">+ Add condition</button>
<div><span class="muted">Dominance hides (#9852):</span> <label><input type="checkbox" data-dom="dominated"> Hide dominated</label> <label><input type="checkbox" data-dom="slower"> Hide slower (beaten)</label> <label><input type="checkbox" data-dom="dumber"> Hide lower-score (beaten)</label> <label><input type="checkbox" data-dom="pricier"> Hide pricier (beaten)</label></div>
</details></div>
<div style="flex-basis:100%">
<label>Settings name <input type="text" id="xf-setup-name" placeholder="Name this setup"></label>
<button type="button" class="xpl-ctl-btn" id="xf-setup-save">Save setup</button>
<label>Saved setups <select id="xf-setup-list"></select></label>
<button type="button" class="xpl-ctl-btn" id="xf-setup-load">Load</button>
<button type="button" class="xpl-ctl-btn" id="xf-setup-export">Export settings</button>
<label class="xpl-ctl-btn">Import <input type="file" id="xf-setup-import" accept="application/json" hidden></label>
<button type="button" class="xpl-ctl-btn" id="xf-setup-reset">Reset all settings</button>
<button type="button" class="xpl-ctl-btn" id="xf-dl-json">Download joined data (JSON)</button>
<button type="button" class="xpl-ctl-btn" id="xf-dl-csv">Download visible CSV</button>
</div>
<p class="muted" id="xp-note" style="flex-basis:100%;margin:2px 0 0"></p>
</div>
<div class="card xpl-detail" id="xpl-detail"></div>
<div class="card" id="xpl-tray" hidden><strong>Hidden</strong> <span id="xpl-traychips"></span></div>
<section id="xtab-aa">
${aaTabAge}
<div id="xpl-legend" class="strip"></div>
<div class="xpl-charts" id="xpl-charts">
<div class="chart" data-chart="xpl-c1"><h3>Completion time × Score</h3><span class="xpl-ctl"><label><input type="checkbox" data-sizeby="xpl-c1"> size dots by cost</label><button type="button" data-dl="xpl-c1">Download PNG</button></span><span class="chart-tools" data-tools="xpl-c1"><button type="button" data-zact="expand" data-zfor="xpl-c1">⤢ Expand</button><button type="button" data-zact="in" data-zfor="xpl-c1" aria-label="Zoom in">+</button><button type="button" data-zact="out" data-zfor="xpl-c1" aria-label="Zoom out">−</button><button type="button" data-zact="reset" data-zfor="xpl-c1" aria-label="Reset view">⟲</button></span><svg id="xpl-c1" viewBox="0 0 680 400" role="img" aria-label="Completion time by score scatter chart"></svg><p class="muted" data-note></p></div>
<div class="chart" data-chart="xpl-c2"><h3>Cost × Score</h3><span class="xpl-ctl"><label><input type="checkbox" data-sizeby="xpl-c2"> size dots by time</label><label><input type="checkbox" data-overlay="trend"> trend</label><label><input type="checkbox" data-overlay="envelope"> envelope</label><label><input type="checkbox" data-overlay="tail"> tail</label><button type="button" data-dl="xpl-c2">Download PNG</button></span><span class="chart-tools" data-tools="xpl-c2"><button type="button" data-zact="expand" data-zfor="xpl-c2">⤢ Expand</button><button type="button" data-zact="in" data-zfor="xpl-c2" aria-label="Zoom in">+</button><button type="button" data-zact="out" data-zfor="xpl-c2" aria-label="Zoom out">−</button><button type="button" data-zact="reset" data-zfor="xpl-c2" aria-label="Reset view">⟲</button></span><svg id="xpl-c2" viewBox="0 0 680 400" role="img" aria-label="Cost by score scatter chart"></svg><p class="muted" data-note></p></div>
<div class="chart" data-chart="xpl-c3"><h3>Cost × Completion time</h3><span class="xpl-ctl"><label><input type="checkbox" data-sizeby="xpl-c3"> size dots by score</label><button type="button" data-dl="xpl-c3">Download PNG</button></span><span class="chart-tools" data-tools="xpl-c3"><button type="button" data-zact="expand" data-zfor="xpl-c3">⤢ Expand</button><button type="button" data-zact="in" data-zfor="xpl-c3" aria-label="Zoom in">+</button><button type="button" data-zact="out" data-zfor="xpl-c3" aria-label="Zoom out">−</button><button type="button" data-zact="reset" data-zfor="xpl-c3" aria-label="Reset view">⟲</button></span><svg id="xpl-c3" viewBox="0 0 680 400" role="img" aria-label="Cost by completion time scatter chart"></svg><p class="muted" data-note></p></div>
<div class="chart" data-chart="xpl-c4"><h3>Tasks completed vs duration</h3><span class="xpl-ctl"><button type="button" data-tmode="per">Seconds per task</button><button type="button" data-tmode="tot">Total time to finish all tasks</button><button type="button" data-dl="xpl-c4">Download PNG</button></span><span class="chart-tools" data-tools="xpl-c4"><button type="button" data-zact="expand" data-zfor="xpl-c4">⤢ Expand</button><button type="button" data-zact="in" data-zfor="xpl-c4" aria-label="Zoom in">+</button><button type="button" data-zact="out" data-zfor="xpl-c4" aria-label="Zoom out">−</button><button type="button" data-zact="reset" data-zfor="xpl-c4" aria-label="Reset view">⟲</button></span><svg id="xpl-c4" viewBox="0 0 680 400" role="img" aria-label="Tasks completed by duration scatter chart"></svg><p class="muted" data-note></p></div>
<div class="chart" data-chart="xpl-c5"><h3>Multiples of the baseline</h3><span class="xpl-ctl"><label>Baseline <select id="xf-baseline"></select></label><button type="button" data-dl="xpl-c5">Download PNG</button></span><span class="chart-tools" data-tools="xpl-c5"><button type="button" data-zact="expand" data-zfor="xpl-c5">⤢ Expand</button><button type="button" data-zact="in" data-zfor="xpl-c5" aria-label="Zoom in">+</button><button type="button" data-zact="out" data-zfor="xpl-c5" aria-label="Zoom out">−</button><button type="button" data-zact="reset" data-zfor="xpl-c5" aria-label="Reset view">⟲</button></span><svg id="xpl-c5" viewBox="0 0 680 400" role="img" aria-label="Cost and duration multiples scatter chart"></svg><p class="muted" data-note></p></div>
<div class="chart" data-chart="xpl-c6"><h3>Scores — every measured configuration</h3><span class="xpl-ctl"><button type="button" data-dl="xpl-c6">Download PNG</button></span><span class="chart-tools" data-tools="xpl-c6"><button type="button" data-zact="expand" data-zfor="xpl-c6">⤢ Expand</button><button type="button" data-zact="in" data-zfor="xpl-c6" aria-label="Zoom in">+</button><button type="button" data-zact="out" data-zfor="xpl-c6" aria-label="Zoom out">−</button><button type="button" data-zact="reset" data-zfor="xpl-c6" aria-label="Reset view">⟲</button></span><svg id="xpl-c6" viewBox="0 0 680 400" role="img" aria-label="Scores bar chart"></svg><p class="muted" data-note></p></div>
</div>
<div class="card" id="xpl-unpriced-wrap"><h3>Scored configurations without a published cost</h3><p class="muted">These visible configurations have a score but no published cost, so they cannot appear on the cost charts. Select one to open its detail card.</p><div id="xpl-unpriced" class="strip"></div></div>
<p class="muted">Time is seconds to finish one task (log scale), not tokens/sec — a model can stream fast and still finish slowly if it burns many tokens, and per-task time here excludes time to first token (shown separately where timed). Thin lines join each model's effort rungs (DeepSeek ladders dotted); the white dashed line is that chart's Pareto frontier; white-ringed dots sit on it, dimmed dots are beaten on its axes. Bracket splits: cost $0.15 / $0.75 / $2.50, time 1m / 5m / 15m. Only measured configurations are plotted — nothing is estimated, so configurations AA never timed appear in the tables, not the time charts. DeepSeek prices split peak / off-peak: peak is 01:00–04:00 and 06:00–10:00 UTC Monday–Friday at double the off-peak token rates, so the same runs appear at two costs. Click a dot or a row for details.</p>
<h2>Multiples table</h2>
<p class="muted">Every visible configuration with all three measures, as multiples of the baseline chosen above, sorted by cost multiple.</p>
<div class="tblwrap"><table class="xpl-table"><caption>Multiples table — every visible configuration as multiples of the baseline</caption><thead><tr><th scope="col">Model</th><th scope="col">Effort</th><th scope="col" class="num">Score</th><th scope="col" class="num">Score ×</th><th scope="col" class="num xpl-costcol">Cost/task</th><th scope="col" class="num xpl-costcol">Cost ×</th><th scope="col" class="num">Time/task</th><th scope="col" class="num">Time ×</th><th scope="col">Benchmark</th></tr></thead><tbody id="xpl-mtrows"></tbody></table></div>
<h2>Configurations</h2>
<p class="muted" id="xpl-count"></p>
<div class="tblwrap"><table class="xpl-table"><caption>Configurations — intelligence, cost, and speed per model</caption><thead><tr><th scope="col" data-sortkey="family" data-dir="asc">Model</th><th scope="col" data-sortkey="effort" data-dir="asc">Effort</th><th scope="col" class="num" id="xpl-scorehead" data-sortkey="score" data-dir="desc">Intelligence</th><th scope="col" class="num xpl-costcol" data-sortkey="cost" data-dir="asc">$/task</th><th scope="col" class="num" data-sortkey="time" data-dir="asc">Time/task</th><th scope="col" class="num" data-sortkey="ttft" data-dir="asc">TTFT</th><th scope="col" class="num" data-sortkey="tps" data-dir="desc">Speed (t/s)</th><th scope="col" class="num xpl-costcol" data-sortkey="perDollar" data-dir="desc">Score / $</th><th scope="col" class="num" data-sortkey="perSecond" data-dir="desc">Score / s</th><th scope="col" class="num xpl-costcol" id="xpl-bcosthead" style="display:none">Score/$ x base</th><th scope="col" class="num" id="xpl-bspeedhead" style="display:none">Score/s x base</th></tr></thead><tbody id="xpl-rows"></tbody></table></div>
<h2>Pareto frontiers</h2>
<p class="muted">Each list keeps variants that are not dominated on its named dimensions: another variant cannot be at least as good on every dimension and strictly better on one. The first two lists are the same score–speed frontier in different priority orders. Percentile ranks use the current filtered models; 100% is best. Missing measures are excluded only from a frontier that needs them.</p>
<label>Minimum score and speed percentile in the price shortlist <select id="xf-pfcut"><option value="50" selected>At least median · 50th percentile</option><option value="60">At least 60th percentile</option><option value="75">Top quarter · 75th percentile</option><option value="0">No cutoff · full Pareto frontier</option></select></label>
<p class="muted" id="xpl-pf-count"></p>
<h3>Score first, with competitive time</h3>
<div class="tblwrap"><table class="xpl-table"><caption>Pareto frontier — score first, with competitive time</caption><thead><tr><th scope="col">Model / effort</th><th scope="col" class="num">Score · value / percentile</th><th scope="col" class="num">Time / task · value / percentile</th><th scope="col" class="num xpl-costcol">Cost / task · value / percentile</th></tr></thead><tbody id="xpl-pf-score"></tbody></table></div>
<h3>Time first, with competitive score</h3>
<div class="tblwrap"><table class="xpl-table"><caption>Pareto frontier — time first, with competitive score</caption><thead><tr><th scope="col">Model / effort</th><th scope="col" class="num">Score · value / percentile</th><th scope="col" class="num">Time / task · value / percentile</th><th scope="col" class="num xpl-costcol">Cost / task · value / percentile</th></tr></thead><tbody id="xpl-pf-speed"></tbody></table></div>
<h3>Price first, strong in score and time</h3>
<div class="tblwrap"><table class="xpl-table"><caption>Pareto frontier — price first, strong in score and time</caption><thead><tr><th scope="col">Model / effort</th><th scope="col" class="num">Score · value / percentile</th><th scope="col" class="num">Time / task · value / percentile</th><th scope="col" class="num xpl-costcol">Cost / task · value / percentile</th></tr></thead><tbody id="xpl-pf-price"></tbody></table></div>
<h3>Price–speed trade-offs</h3>
<div class="tblwrap"><table class="xpl-table"><caption>Pareto frontier — price and speed trade-offs</caption><thead><tr><th scope="col">Model / effort</th><th scope="col" class="num">Time / task</th><th scope="col" class="num xpl-costcol">Cost / task</th><th scope="col" class="num">Score · context</th></tr></thead><tbody id="xpl-pf-ps"></tbody></table></div>
<h2>Recommendations</h2>
<p class="muted">Build a useful shortlist: first require a near-top score in each band, then trade price against speed and choose distinct labs. Score bands are non-overlapping and calculated from the filtered population before tradeoff limits; absolute limits apply first, then relative price/speed limits against the cheapest and fastest near-top models passing them. Unknown selected values cannot be recommended.</p>
<div class="card xpl-filters">
<span role="group" aria-label="Recommendation presets"><button type="button" class="xpl-ctl-btn" data-recpreset="fast">Fast near-top</button><button type="button" class="xpl-ctl-btn" data-recpreset="balanced">Balanced</button><button type="button" class="xpl-ctl-btn" data-recpreset="diversity">More lab diversity</button><button type="button" class="xpl-ctl-btn" data-recpreset="budget">Tight budget</button><button type="button" class="xpl-ctl-btn" data-recpreset="reset">Reset tradeoffs</button></span>
<label>Score bands <select id="xf-bands"><option value="2">2 tiers</option><option value="3" selected>3 tiers</option><option value="4">4 tiers</option><option value="5">5 tiers</option><option value="6">6 tiers</option><option value="8">8 tiers</option></select></label>
<label>Allow score below band best <input type="number" data-rec="allow" value="5" min="0" step="1" style="width:4em"> points</label>
<label>Favor price ← → favor speed <input type="range" data-rec="speedPriority" min="0" max="100" step="5" value="85"> <span id="rec-speedpriority-val">85% speed</span></label>
<label>Extra price above cheapest near-top <input type="number" data-rec="priceAllow" value="500" min="0" step="50" style="width:5em"> %</label>
<label>Slowest allowed vs fastest near-top <input type="number" data-rec="slowAllow" value="2" min="1" step="0.1" style="width:4em"> ×</label>
<label>Allow alternatives within <input type="number" data-rec="altWindow" value="2" min="1" step="0.1" style="width:4em"> × best tradeoff</label>
<label>Lab diversity <select data-rec="diversity"><option value="lab" selected>One choice per lab</option><option value="labsfirst">Different labs first</option><option value="none">No lab grouping</option></select></label>
<label>Remove dominated models <select data-rec="dominance"><option value="lab" selected>Within each lab</option><option value="all">Across all labs · strict Pareto</option><option value="none">Do not remove</option></select></label>
<label>At most per band <select data-rec="perBand"><option value="1">1</option><option value="2">2</option><option value="3" selected>3</option><option value="4">4</option><option value="5">5</option></select></label>
<label>Maximum USD / task <input type="number" data-rec="maxCost" placeholder="none" min="0" step="0.1" style="width:5em"></label>
<label>Maximum seconds / task <input type="number" data-rec="maxSec" placeholder="none" min="0" step="10" style="width:5em"></label>
<button type="button" class="xpl-ctl-btn" id="xpl-rec-export">Export shortlist CSV</button>
</div>
<p class="muted" id="xpl-rec-summary"></p>
<div id="xpl-recs"></div>
<h2>Band comparisons</h2>
<p class="muted">Best choices within each score and cost band; shortlists follow the current model and effort filters. Each band row shows the observed range of its other two measures with the endpoint configurations named.</p>
<label>Near-leader window <select id="xf-bandscale"><option value="0.5">0.5× · Very tight</option><option value="0.75">0.75× · Tight</option><option value="1" selected>1× · Standard (score ±2 · time within 10% · price within 25%)</option><option value="1.5">1.5× · Generous</option><option value="2">2× · Wide</option><option value="3">3× · Broad</option></select></label>
<div id="xpl-bandgroups"></div>
<h2>Timing — Test_Runs</h2>
<p class="muted">Per-configuration timing from the import's Test_Runs data. "Not measured" means the import carries no value for that cell — nothing is estimated.</p>
${aaTimingsTable(s.testRuns)}
<h2>Measured configurations</h2>
<p class="muted">The AA Intelligence Index rows of the latest import batch, effort labels verbatim from the import.</p>
${aaConfigsTable(s.configs)}
${s.cai ? aaCaiTable(s.cai) : ""}
</section>
<section id="xtab-models" hidden>
${catalogTabAge}
<h2>The fleet's model universe</h2>
<p class="muted">Every model in the fleet config's provider catalogs: the plan each door belongs to and its effective value multiplier where the registry records one, the pools that seat it, and its AA measurement when the import has one. Click a row for the plan note.</p>
${modelCards}
<h3>Task projection — estimated, not measured</h3>
<p class="muted">The task-assumptions projection from the ChatGPT coding explorer: set the shape of a task and every model with a recorded token rate gets an estimated cost and time, marked ~. Prices come only from the ainventory price event log joined onto this catalog (the band your per-turn input falls in); a model with no recorded rate — or missing a rate the assumptions need — shows —, never a guess. Time borrows the family's measured TTFT and throughput. Cache-storage hours are not modelled: the price log carries no storage rate.</p>
<div class="card xpl-filters">
<span role="group" aria-label="Task presets"><button type="button" class="xpl-ctl-btn" data-pjpreset="small">Small patch</button><button type="button" class="xpl-ctl-btn" data-pjpreset="feature">Feature</button><button type="button" class="xpl-ctl-btn" data-pjpreset="repo">Repo task</button></span>
<label>Turns <input type="number" data-pj="turns" value="3" min="1" style="width:4em"></label>
<label>Input / turn <input type="number" data-pj="inputTokens" value="20000" step="1000" style="width:6em"></label>
<label>Answer / turn <input type="number" data-pj="answerTokens" value="2000" step="500" style="width:5em"></label>
<label>Thinking / turn <input type="number" data-pj="thinkingTokens" value="4000" step="500" style="width:5em"></label>
<label>Cache read share <input type="range" data-pj="cacheReadPct" min="0" max="100" step="5" value="50"> <span id="pj-cacheread-val">50%</span></label>
<label>Writes / uncached <input type="number" data-pj="writesPct" value="0" min="0" max="100" style="width:4em"> %</label>
<label>Tool overhead / turn <input type="number" data-pj="overheadSec" value="10" min="0" style="width:4em"> s</label>
<label>Rate window <select data-pj="window"><option value="peak" selected>Peak (conservative)</option><option value="offpeak">Off-peak</option></select></label>
<span class="muted">The window applies to scheduled rates only (DeepSeek halves every token rate off-peak).</span>
</div>
<p class="muted" id="xpl-mcount"></p>
<div class="tblwrap"><table class="xpl-table"><caption>Model catalog — providers, plans, and projected costs</caption><thead><tr><th scope="col">Model</th><th scope="col">Provider / plan</th><th scope="col" class="num">Context</th><th scope="col">Variants</th><th scope="col">Pool seats</th><th scope="col" class="num">AA score</th><th scope="col" class="num xpl-costcol">AA $/task</th><th scope="col" class="num xpl-costcol">Eff. $/task</th><th scope="col" class="num">AA time/task</th><th scope="col" class="num xpl-costcol">Proj. $/task ~</th><th scope="col" class="num">Proj. time/task ~</th><th scope="col" class="num xpl-costcol">AA score / proj. $</th></tr></thead><tbody id="xpl-mrows"></tbody></table></div>
</section>
${coverageHtml}
<script type="application/json" id="xpl-data">${explorerJson}</script>`;
  return layout("Cost Explorer", body, provenance, "/aa", AA_EXPLORER_SCRIPT, aaFreshnessChips(detail, section));
}
