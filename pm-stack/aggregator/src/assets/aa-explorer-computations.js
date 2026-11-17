// Pure computations shared by the Cost Explorer browser client and Bun tests.
// Keep this file browser-safe: it is served as-is and has no DOM dependencies.

export function pctRank(vals, v, higherBetter) {
  var xs = vals.filter(function (x) { return x != null; });
  if (xs.length < 2 || v == null) return null;
  var worse = 0, ties = 0;
  xs.forEach(function (x) {
    if (x === v) ties++;
    else if (higherBetter ? x < v : x > v) worse++;
  });
  return Math.round((100 * (worse + 0.5 * (ties - 1))) / (xs.length - 1));
}

export function paretoSet(rows, dims) {
  var pool = rows.filter(function (p) { return dims.every(function (d) { return d.get(p) != null; }); });
  return pool.filter(function (p) {
    return !pool.some(function (q) {
      if (q === p) return false;
      var allGe = true, anyGt = false;
      dims.forEach(function (d) {
        var a = d.get(q), b = d.get(p);
        if (d.better === "high") { if (a < b) allGe = false; if (a > b) anyGt = true; }
        else { if (a > b) allGe = false; if (a < b) anyGt = true; }
      });
      return allGe && anyGt;
    });
  });
}

export function gapMetrics(rows) {
  var map = {};
  rows.forEach(function (p) {
    var g = { score: p.score, cost: p.cost, time: p.time, tps: p.tps, gapScore: null, gapCost: null, gapSpeed: null };
    if (p.cost != null && p.score != null) {
      var best = null, minCost = null;
      rows.forEach(function (q) {
        if (q.cost == null || q.score == null) return;
        if (q.cost <= p.cost && (best == null || q.score > best)) best = q.score;
        if (q.score >= p.score && (minCost == null || q.cost < minCost)) minCost = q.cost;
      });
      if (best != null) g.gapScore = best - p.score;
      if (minCost != null && minCost > 0) g.gapCost = p.cost / minCost;
    }
    if (p.time != null && p.score != null) {
      var bestT = null;
      rows.forEach(function (q) {
        if (q.time == null || q.score == null) return;
        if (q.time <= p.time && (bestT == null || q.score > bestT)) bestT = q.score;
      });
      if (bestT != null) g.gapSpeed = bestT - p.score;
    }
    map[p.key] = g;
  });
  return map;
}

var INEQ_OPS = {
  "<": function (a, b) { return a < b; },
  "<=": function (a, b) { return a <= b; },
  "=": function (a, b) { return a === b; },
  ">=": function (a, b) { return a >= b; },
  ">": function (a, b) { return a > b; },
};

export function applyInequalityFilters(rows, mode, combine, conditions) {
  if (mode === "off") return rows;
  var active = conditions.filter(function (c) { return c.value !== "" && !isNaN(Number(c.value)); });
  if (!active.length) return rows;
  var gaps = gapMetrics(rows);
  return rows.filter(function (p) {
    var g = gaps[p.key];
    var results = active.map(function (c) {
      var v = g ? g[c.metric] : null;
      if (v == null || !INEQ_OPS[c.op]) return false;
      return INEQ_OPS[c.op](v, Number(c.value));
    });
    var match = combine === "or" ? results.some(Boolean) : results.every(Boolean);
    return mode === "reject" ? !match : match;
  });
}

export function logEdges(vals, n) {
  var xs = vals.filter(function (v) { return v != null && v > 0; });
  if (!xs.length) return [];
  var mn = Math.min.apply(null, xs), mx = Math.max.apply(null, xs);
  if (mn === mx) return [mn, mx];
  var edges = [];
  for (var i = 0; i <= n; i++) edges.push(mn * Math.pow(mx / mn, i / n));
  return edges;
}

export function scoreEdges(rows, n) {
  var xs = rows.map(function (p) { return p.score; }).filter(function (v) { return v != null; });
  if (!xs.length) return [];
  var mn = Math.min.apply(null, xs), mx = Math.max.apply(null, xs);
  var edges = [];
  for (var i = 0; i <= n; i++) edges.push(mn + ((mx - mn) * i) / n);
  return edges;
}

export function bandIndex(v, edges) {
  for (var i = 0; i < edges.length - 1; i++) {
    if (v >= edges[i] && (i === edges.length - 2 ? v <= edges[i + 1] : v < edges[i + 1])) return i;
  }
  return -1;
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

function fmtN(v, d) {
  return v == null ? "—" : String(Math.round(v * Math.pow(10, d)) / Math.pow(10, d));
}

export function computeRecommendations(rows, R, bandCount, scoreName) {
  var scored = rows.filter(function (p) { return p.score != null; });
  if (!scored.length) return { bands: [], empty: "No scored configurations are visible under the current filters." };
  var lo = Math.min.apply(null, scored.map(function (p) { return p.score; }));
  var hi = Math.max.apply(null, scored.map(function (p) { return p.score; }));
  var n = Math.max(2, Math.min(8, Number(bandCount) || 3));
  var w = (hi - lo) / n || 1;
  var bands = [];
  for (var i = 0; i < n; i++) {
    var bLo = lo + i * w, bHi = i === n - 1 ? hi : lo + (i + 1) * w;
    var members = scored.filter(function (p) { return p.score >= bLo && (i === n - 1 ? p.score <= bHi : p.score < bHi); });
    if (!members.length) continue;
    var best = Math.max.apply(null, members.map(function (p) { return p.score; }));
    var decision = {};
    var candidates = [];
    members.forEach(function (p) {
      if (p.cost == null || p.time == null) decision[p.key] = "Missing selected cost or speed";
      else candidates.push(p);
    });
    var ref = candidates.slice().sort(function (a, b) { return cmpDesc(a.score, b.score) || cmpAsc(a.cost, b.cost); })[0] || null;
    var maxCost = R.maxCost === "" || R.maxCost == null ? null : Number(R.maxCost);
    var maxTime = R.maxSec === "" || R.maxSec == null ? null : Number(R.maxSec);
    var nearTop = [];
    candidates.forEach(function (p) {
      if (p.score < best - Number(R.allow)) decision[p.key] = "Below near-top score floor";
      else if (maxCost != null && p.cost > maxCost) decision[p.key] = "Above the absolute price limit";
      else if (maxTime != null && p.time > maxTime) decision[p.key] = "Above the absolute time limit";
      else nearTop.push(p);
    });
    var picks = [];
    var cheapest = null, fastest = null, priceCeil = null, timeCeil = null;
    if (nearTop.length) {
      cheapest = Math.min.apply(null, nearTop.map(function (p) { return p.cost; }));
      fastest = Math.min.apply(null, nearTop.map(function (p) { return p.time; }));
      priceCeil = cheapest * (1 + Number(R.priceAllow) / 100);
      timeCeil = fastest * Number(R.slowAllow);
      var survivors = [];
      nearTop.forEach(function (p) {
        if (p.cost > priceCeil) decision[p.key] = "Above relative price allowance";
        else if (p.time > timeCeil) decision[p.key] = "Slower than relative speed limit";
        else survivors.push(p);
      });
      var kept = survivors.filter(function (p) {
        if (R.dominance === "none") return true;
        var dominated = survivors.some(function (q) {
          if (q === p) return false;
          if (R.dominance === "lab" && q.creator !== p.creator) return false;
          return q.score >= p.score && q.cost <= p.cost && q.time <= p.time && (q.score > p.score || q.cost < p.cost || q.time < p.time);
        });
        if (dominated) decision[p.key] = R.dominance === "lab" ? "Dominated within its lab" : "Dominated on score, price and speed";
        return !dominated;
      });
      var sw = Number(R.speedPriority) / 100;
      var ranked = kept.map(function (p) {
        return { p: p, factor: Math.pow(p.time / fastest, sw) * Math.pow(p.cost / cheapest, 1 - sw) };
      }).sort(function (a, b) { return a.factor - b.factor || cmpDesc(a.p.score, b.p.score); });
      var bestFactor = ranked.length ? ranked[0].factor : null;
      var pickedLabs = {};
      var passes = R.diversity === "labsfirst" ? 2 : 1;
      for (var pass = 0; pass < passes && picks.length < Number(R.perBand); pass++) {
        ranked.forEach(function (r) {
          if (picks.length >= Number(R.perBand)) return;
          if (picks.some(function (x) { return x.p.key === r.p.key; })) return;
          if (bestFactor != null && r.factor > bestFactor * Number(R.altWindow)) return;
          if (R.diversity === "lab" && pickedLabs[r.p.creator]) return;
          if (R.diversity === "labsfirst" && pass === 0 && pickedLabs[r.p.creator]) return;
          picks.push(r);
          pickedLabs[r.p.creator] = true;
        });
      }
      ranked.forEach(function (r) {
        if (picks.some(function (x) { return x.p.key === r.p.key; })) { decision[r.p.key] = "Recommended"; return; }
        if (decision[r.p.key]) return;
        if (bestFactor != null && r.factor > bestFactor * Number(R.altWindow)) decision[r.p.key] = "Outside the alternative window";
        else if (R.diversity === "lab" && pickedLabs[r.p.creator]) decision[r.p.key] = "Not selected — one choice per lab";
        else decision[r.p.key] = "Not selected — band cap reached";
      });
    }
    bands.push({
      label: "Score band " + (bands.length + 1) + " · " + scoreName + " " + fmtN(bLo, 2) + (i === n - 1 ? " to ≤ " : " to < ") + fmtN(bHi, 2),
      members: members, best: best, ref: ref, picks: picks, decision: decision,
      priceCeil: priceCeil, timeCeil: timeCeil, floor: best - Number(R.allow),
    });
  }
  return { bands: bands, empty: null };
}

export function normName(s) {
  var t = String(s || "").toLowerCase().replace(/[^a-z0-9]+/g, " ").trim();
  for (var i = 0; i < 3; i++) t = t.replace(/(\d) (\d)/g, "$1.$2");
  return t.replace(/\s+/g, " ");
}

function pjBand(e, settings) {
  if (!e.rate || !e.rate.bands.length) return null;
  var bands = e.rate.bands;
  var win = bands.filter(function (b) { return b.window === settings.window; });
  var pool = win.length ? win : bands.filter(function (b) { return b.window == null || b.window === "flat"; });
  if (!pool.length) pool = bands;
  var input = Number(settings.inputTokens) || 0;
  for (var i = 0; i < pool.length; i++) {
    var b = pool[i];
    if ((b.minTokens == null || input >= b.minTokens) && (b.maxTokens == null || input < b.maxTokens)) return b;
  }
  return pool[pool.length - 1];
}

function pjTiming(e, aaPoints, caiPoints) {
  var norm = normName(e.name);
  var sets = [aaPoints, caiPoints];
  for (var si = 0; si < sets.length; si++) {
    var cands = sets[si].filter(function (p) { return normName(p.family) === norm; });
    for (var i = 0; i < cands.length; i++) if (cands[i].tps != null) return { tps: cands[i].tps, ttft: cands[i].ttft };
  }
  return null;
}

export function projectTask(e, settings, aaPoints, caiPoints) {
  var band = pjBand(e, settings);
  if (!band) return { cost: null, time: null, band: null };
  var input = Number(settings.inputTokens) || 0, answer = Number(settings.answerTokens) || 0, thinking = Number(settings.thinkingTokens) || 0;
  var turns = Number(settings.turns) || 0;
  var cacheShare = (Number(settings.cacheReadPct) || 0) / 100;
  var writesShare = (Number(settings.writesPct) || 0) / 100;
  var uncached = input * (1 - cacheShare);
  var reads = input * cacheShare;
  var writes = uncached * writesShare;
  var missing = band.input == null || band.output == null || (reads > 0 && band.cacheRead == null) || (writes > 0 && band.cacheWrite == null);
  var cost = null;
  if (!missing) {
    var perTurn = (uncached * band.input + reads * (band.cacheRead || 0) + writes * (band.cacheWrite || 0) + (answer + thinking) * band.output) / 1000000;
    cost = turns * perTurn;
  }
  var timing = pjTiming(e, aaPoints, caiPoints);
  var time = null;
  if (timing && timing.tps) time = turns * ((timing.ttft || 0) + answer / timing.tps + (Number(settings.overheadSec) || 0));
  return { cost: cost, time: time, band: band };
}
