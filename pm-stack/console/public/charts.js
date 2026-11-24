// Charts, for any screen that has numbers over time: columns stacked by
// what they are made of, lines, and the tile that is one number.
//
// The rules they are drawn by (the same on every screen, so a chart
// here is read like a chart there):
//
//   - colour says which series a mark belongs to and nothing else. The
//     series colours are the theme's own (--series-1 to --series-6 in
//     app.css, a fixed order chosen so that neighbours can be told
//     apart by people who see colour differently); "everything else" is
//     always grey. A series keeps its colour whatever else is shown.
//   - words and numbers are never written in a series colour: a mark of
//     the colour sits beside them.
//   - two or more series always have a legend, with each one's total.
//   - one scale to a chart. Two measures are two charts.
//   - a column is at most 24 wide, has a gap from its neighbour, a gap
//     between its parts, and a rounded top; a line is 2 wide.
//   - pointing at, tapping or arrowing to a day says that day's numbers
//     in the line under the title: every series at once, the number
//     first. Nothing is only in that line: the table under each chart
//     has every number.
//   - today is not over, so its column is drawn fainter.
//
// The sums are plain functions (tested by hand-worked numbers); the
// drawing is SVG elements with classes. Nothing writes a style.
import { h, s } from "./ui.js";

/** The drawing's own units: it is scaled to the width it is given. */
const W = 360;
const H = 172;
const PLOT = { x: 34, y: 10, w: 320, h: 128 };
const GAP = 2;
const BAR_MAX = 24;

/** The smallest clean number (1, 2, 2.5 or 5 times a power of ten) that
 * is at least `value`: the top of a scale. Nothing to show is a scale
 * to 1 (a chart of zeros still has an axis). */
export function niceMax(value) {
  if (!(value > 0)) return 1;
  const power = 10 ** Math.floor(Math.log10(value));
  for (const step of [1, 2, 2.5, 5, 10]) {
    if (step * power >= value) return step * power;
  }
  return 10 * power;
}

/** A count as it is written: in full under ten thousand, then 12.9K, 4.2M. */
export function compact(n) {
  if (n === null || n === undefined || Number.isNaN(n)) return "–";
  const abs = Math.abs(n);
  if (abs < 10_000) return Number.isInteger(n) ? n.toLocaleString("en-US") : String(Math.round(n * 10) / 10);
  if (abs < 1_000_000) return `${Math.round(n / 100) / 10}K`;
  return `${Math.round(n / 100_000) / 10}M`;
}

/** Minutes as a person says them: 45 min, 3.5 h, 2.1 d. */
export function minutesSaid(minutes) {
  if (minutes === null || minutes === undefined) return "–";
  if (minutes < 90) return `${Math.round(minutes)} min`;
  if (minutes < 36 * 60) return `${Math.round(minutes / 6) / 10} h`;
  return `${Math.round(minutes / 144) / 10} d`;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
/** "24 Nov", from a date (the days here are UTC days). */
export const dayShort = (date) => `${Number(date.slice(8, 10))} ${MONTHS[Number(date.slice(5, 7)) - 1]}`;
/** "Tue 24 Nov". */
export const dayLong = (date) => `${WEEKDAYS[new Date(`${date}T00:00:00Z`).getUTCDay()]} ${dayShort(date)}`;

/** The days in groups of `size`, counted back from the last day so that
 * the newest group is whole; the oldest may be short. Each group is the
 * places of its days in the list. A size of 1 is the days themselves. */
export function groupsOf(count, size) {
  const groups = [];
  for (let end = count; end > 0; end -= size) groups.unshift([Math.max(0, end - size), end]);
  return groups;
}

/** A series added up group by group. */
export const rollUp = (values, groups) => groups.map(([from, to]) => values.slice(from, to).reduce((sum, v) => sum + (v ?? 0), 0));

/** The total of a series over its last `days` places, and over the same
 * number before them (null when there are not that many before). */
export function windowTotals(values, days) {
  const n = values.length;
  const sum = (from, to) => values.slice(Math.max(0, from), Math.max(0, to)).reduce((total, v) => total + (v ?? 0), 0);
  return { now: sum(n - days, n), before: n >= days * 2 ? sum(n - days * 2, n - days) : null };
}

/** How a total stands against the one before it. `tone` is "up",
 * "down" or "" (level, or nothing to set it against). */
export function deltaSaid(now, before) {
  if (before === null || before === undefined) return { text: "", tone: "" };
  if (before === 0) return now === 0 ? { text: "level", tone: "" } : { text: `▲ from none`, tone: "up" };
  const change = Math.round(((now - before) / before) * 100);
  if (change === 0) return { text: "level", tone: "" };
  return change > 0 ? { text: `▲ ${change}%`, tone: "up" } : { text: `▼ ${-change}%`, tone: "down" };
}

/** Where each column of a stacked chart goes and how tall each of its
 * parts is. `series` is a list of lists of numbers, bottom first; `max`
 * is the top of the scale. A part with something in it is at least 1
 * tall, and every part but the top one gives up `GAP` to the gap above
 * it. */
export function stackGeometry(series, max, plot = PLOT) {
  const count = series[0]?.length ?? 0;
  const band = count > 0 ? plot.w / count : plot.w;
  const width = Math.max(1, Math.min(BAR_MAX, band - GAP));
  const columns = [];
  for (let i = 0; i < count; i++) {
    const present = series.map((values, k) => [k, values[i] ?? 0]).filter(([, v]) => v > 0);
    let cursor = plot.y + plot.h;
    const parts = present.map(([k, value], at) => {
      const raw = (value / max) * plot.h;
      const top = at === present.length - 1;
      const height = Math.max(1, raw - (top ? 0 : GAP));
      const part = { series: k, value, y: cursor - height, h: height, top };
      cursor -= raw;
      return part;
    });
    columns.push({ x: plot.x + i * band + (band - width) / 2, w: width, total: present.reduce((sum, [, v]) => sum + v, 0), parts });
  }
  return { band, columns };
}

/** The points of a line: one for each place that has a number. A place
 * with none (null) breaks the line there, so the result is a list of
 * runs. */
export function lineRuns(values, max, plot = PLOT) {
  const count = values.length;
  const step = count > 1 ? plot.w / (count - 1) : 0;
  const runs = [];
  let run = [];
  values.forEach((value, i) => {
    if (value === null || value === undefined) {
      if (run.length) runs.push(run);
      run = [];
      return;
    }
    run.push({ i, x: plot.x + (count > 1 ? i * step : plot.w / 2), y: plot.y + plot.h - (Math.min(value, max) / max) * plot.h });
  });
  if (run.length) runs.push(run);
  return runs;
}

/** Which place a pointer is over: its x across the drawing, as a share
 * of the drawing's width, to the nearest place. */
export function placeAt(share, count, plot = PLOT, between = true) {
  if (count <= 0) return -1;
  const x = share * W - plot.x;
  const at = between ? Math.floor(x / (plot.w / count)) : Math.round(count > 1 ? x / (plot.w / (count - 1)) : 0);
  return Math.max(0, Math.min(count - 1, at));
}

// ---- drawing ------------------------------------------------------------

const slotClass = (slot) => (slot === "other" ? "so" : `s${slot}`);

/** A column's part with a rounded top (the top of its column). */
function roundedTop(x, y, w, hh) {
  const r = Math.min(4, w / 2, hh);
  return `M${x},${y + hh}V${y + r}Q${x},${y} ${x + r},${y}H${x + w - r}Q${x + w},${y} ${x + w},${y + r}V${y + hh}Z`;
}

/** The scale's lines and numbers, and the days along the bottom. */
function frame(max, labels, between, format = compact) {
  const out = [];
  for (const share of [0, 0.5, 1]) {
    const y = PLOT.y + PLOT.h - share * PLOT.h;
    out.push(s("line", { class: "grid", x1: PLOT.x, x2: PLOT.x + PLOT.w, y1: y, y2: y }));
    out.push(s("text", { class: "tick", x: PLOT.x - 5, y: y + 3.5, "text-anchor": "end" }, format(max * share)));
  }
  const count = labels.length;
  const at = (i) => (between ? PLOT.x + (i + 0.5) * (PLOT.w / count) : PLOT.x + (count > 1 ? (i * PLOT.w) / (count - 1) : PLOT.w / 2));
  const shown = count <= 1 ? [0] : count <= 8 ? [0, count - 1] : [0, Math.floor((count - 1) / 2), count - 1];
  for (const i of shown) {
    const anchor = i === 0 ? "start" : i === count - 1 ? "end" : "middle";
    const x = i === 0 ? PLOT.x : i === count - 1 ? PLOT.x + PLOT.w : at(i);
    out.push(s("text", { class: "tick", x, y: H - 8, "text-anchor": anchor }, labels[i]));
  }
  return out;
}

/** The line under a chart's title: what is pointed at, every series at once. */
function readoutLine() {
  const el = h("p", { class: "readout", role: "status", "aria-live": "polite" });
  const say = (lead, items) => {
    // A space between each, so that read aloud they are separate words
    // (on the screen the row's own gap keeps them apart).
    el.replaceChildren(
      h("b", {}, lead),
      ...items.flatMap((item) => [" ", h("span", { class: "read" }, item.slot === undefined ? null : h("i", { class: `sw ${slotClass(item.slot)}` }), h("b", {}, item.value), ` ${item.name}`)]),
    );
  };
  return { el, say };
}

/** The names of the series, each with a mark of its colour and its total. */
function legend(series, line) {
  return h(
    "ul",
    { class: "legend" },
    series.map((one) => h("li", {}, h("i", { class: `sw ${line ? "ln " : ""}${slotClass(one.slot)}` }), `${one.name} `, h("b", {}, one.total))),
  );
}

/** Every number of a chart, as a table: what the chart shows, for a
 * reader who cannot see it or wants the figures. */
function tableOf(labels, series, format = compact) {
  return h(
    "details",
    { class: "figures" },
    h("summary", {}, "The numbers"),
    h(
      "div",
      { class: "scroll" },
      h(
        "table",
        {},
        h("thead", {}, h("tr", {}, h("th", { scope: "col" }, "When"), series.map((one) => h("th", { scope: "col" }, one.name)))),
        h(
          "tbody",
          {},
          labels.map((label, i) => h("tr", {}, h("th", { scope: "row" }, label), series.map((one) => h("td", {}, format(one.values[i]))))),
        ),
      ),
    ),
  );
}

/** Pointing, tapping and the arrow keys choose a place; leaving lets go. */
function choosing(svg, count, between, choose) {
  let at = -1;
  const set = (next) => {
    if (next === at) return;
    at = next;
    choose(at);
  };
  const from = (event) => {
    const box = svg.getBoundingClientRect();
    if (box.width <= 0) return;
    set(placeAt((event.clientX - box.left) / box.width, count, PLOT, between));
  };
  svg.addEventListener("pointermove", from);
  svg.addEventListener("pointerdown", from);
  svg.addEventListener("pointerleave", (event) => {
    // A finger lifted leaves the day it chose on show; a mouse moved away lets go.
    if (event.pointerType === "mouse") set(-1);
  });
  svg.addEventListener("blur", () => set(-1));
  svg.addEventListener("keydown", (event) => {
    const move = { ArrowLeft: -1, ArrowRight: 1 }[event.key];
    if (move !== undefined) set(Math.max(0, Math.min(count - 1, (at < 0 ? (move > 0 ? -1 : count) : at) + move)));
    else if (event.key === "Home") set(0);
    else if (event.key === "End") set(count - 1);
    else if (event.key === "Escape") set(-1);
    else return;
    event.preventDefault();
  });
}

/** Columns stacked by what they are made of.
 *
 *   title    what the chart is of
 *   labels   one for each column, short ("24 Nov")
 *   long     one for each column, as said in the line under the title
 *   series   [{ name, slot, values }], bottom first; slot is 1 to 6, or "other"
 *   unit     what a column's total is of ("merged")
 *   partial  the last column is not over yet (today)
 */
export function stackedColumns({ title, note = "", labels, long, series, unit, partial = false }) {
  const shown = series.filter((one) => one.values.some((v) => v > 0));
  const drawn = shown.length > 0 ? shown : series.slice(0, 1);
  const totals = labels.map((_, i) => drawn.reduce((sum, one) => sum + (one.values[i] ?? 0), 0));
  const max = niceMax(Math.max(0, ...totals));
  const { columns } = stackGeometry(
    drawn.map((one) => one.values),
    max,
  );
  const readout = readoutLine();
  const whole = totals.reduce((sum, v) => sum + v, 0);
  const rest = () => readout.say(`${compact(whole)} ${unit}`, []);
  const groups = columns.map((column, i) =>
    s(
      "g",
      { class: `col${partial && i === columns.length - 1 ? " partial" : ""}` },
      column.parts.map((part) =>
        part.top
          ? s("path", { class: slotClass(drawn[part.series].slot), d: roundedTop(column.x, part.y, column.w, part.h) })
          : s("rect", { class: slotClass(drawn[part.series].slot), x: column.x, y: part.y, width: column.w, height: part.h }),
      ),
    ),
  );
  const svg = s(
    "svg",
    { class: "plot", viewBox: `0 0 ${W} ${H}`, role: "img", tabindex: "0", "aria-label": `${title}. ${compact(whole)} ${unit} in all. The arrow keys read each column.` },
    frame(max, labels, true),
    groups,
  );
  choosing(svg, columns.length, true, (at) => {
    groups.forEach((g, i) => g.classList.toggle("on", i === at));
    svg.classList.toggle("choosing", at >= 0);
    if (at < 0) return rest();
    readout.say(
      `${long[at]} · ${compact(totals[at])}${partial && at === columns.length - 1 ? " so far" : ""}`,
      drawn.length > 1 ? drawn.map((one) => ({ slot: one.slot, value: compact(one.values[at] ?? 0), name: one.name })) : [],
    );
  });
  rest();
  const withTotals = drawn.map((one) => ({ ...one, total: compact(one.values.reduce((sum, v) => sum + (v ?? 0), 0)) }));
  return h(
    "figure",
    { class: "card chart" },
    h("figcaption", {}, h("h3", {}, title), note ? h("p", { class: "meta" }, note) : null),
    readout.el,
    svg,
    drawn.length > 1 ? legend(withTotals, false) : null,
    tableOf(long, drawn.length > 1 ? [...drawn, { name: "All", values: totals }] : drawn),
  );
}

/** Lines over time. One series is drawn with a wash under it and needs
 * no legend (the title says what it is); `format` writes its numbers. */
export function lineChart({ title, note = "", labels, long, series, format = compact, unit = "" }) {
  const all = series.flatMap((one) => one.values).filter((v) => v !== null && v !== undefined);
  const max = niceMax(Math.max(0, ...all));
  const readout = readoutLine();
  // At rest: each series' figure for the whole of what is shown, where it was given one.
  const rest = () =>
    readout.say(
      `${labels.length} days`,
      series.filter((one) => one.total !== undefined).map((one) => ({ slot: one.slot, value: one.total, name: series.length > 1 ? one.name : unit })),
    );
  const marks = [];
  const dots = [];
  for (const one of series) {
    const runs = lineRuns(one.values, max);
    const cls = slotClass(one.slot);
    for (const run of runs) {
      const d = run.map((p, i) => `${i ? "L" : "M"}${Math.round(p.x * 100) / 100},${Math.round(p.y * 100) / 100}`).join("");
      if (series.length === 1 && run.length > 1) {
        const base = PLOT.y + PLOT.h;
        marks.push(s("path", { class: `wash ${cls}`, d: `${d}L${run[run.length - 1].x},${base}L${run[0].x},${base}Z` }));
      }
      // A run of one day is a dot; of more, a line.
      if (run.length > 1) marks.push(s("path", { class: `ln ${cls}`, d }));
      else marks.push(s("circle", { class: `dot ${cls}`, cx: run[0].x, cy: run[0].y, r: 2.5 }));
    }
    // The dot that follows the place chosen; it starts on the last number.
    const lastRun = runs[runs.length - 1];
    const end = lastRun ? lastRun[lastRun.length - 1] : null;
    const dot = s("circle", { class: `dot ring ${cls}`, cx: end ? end.x : -20, cy: end ? end.y : -20, r: 4 });
    dots.push({ dot, runs, end });
    marks.push(dot);
  }
  const hair = s("line", { class: "hair", x1: -9, x2: -9, y1: PLOT.y, y2: PLOT.y + PLOT.h });
  const svg = s(
    "svg",
    { class: "plot", viewBox: `0 0 ${W} ${H}`, role: "img", tabindex: "0", "aria-label": `${title}. The arrow keys read each day.` },
    frame(max, labels, false, format),
    hair,
    marks,
  );
  choosing(svg, labels.length, false, (at) => {
    for (const { dot, runs, end } of dots) {
      const point = at < 0 ? end : runs.flat().find((p) => p.i === at);
      dot.setAttribute("cx", String(point ? point.x : -20));
      dot.setAttribute("cy", String(point ? point.y : -20));
    }
    const x = at < 0 ? -9 : PLOT.x + (labels.length > 1 ? (at * PLOT.w) / (labels.length - 1) : PLOT.w / 2);
    hair.setAttribute("x1", String(x));
    hair.setAttribute("x2", String(x));
    if (at < 0) return rest();
    readout.say(
      long[at],
      series.map((one) => ({ slot: one.slot, value: format(one.values[at]), name: series.length > 1 ? one.name : unit })),
    );
  });
  rest();
  return h(
    "figure",
    { class: "card chart" },
    h("figcaption", {}, h("h3", {}, title), note ? h("p", { class: "meta" }, note) : null),
    readout.el,
    svg,
    series.length > 1 ? legend(series, true) : null,
    tableOf(long, series, format),
  );
}

/** A row of the numbers a screen leads with. Each: what it is, the
 * number, how it stands against the time before (an arrow and a figure,
 * never colour alone), and the shape of its days. */
export function statTiles(tiles) {
  return h(
    "div",
    { class: "kpis" },
    tiles.map((tile) => {
      const values = tile.spark ?? [];
      const max = niceMax(Math.max(0, ...values));
      const plot = { x: 1, y: 2, w: 98, h: 20 };
      const run = lineRuns(values, max, plot)[0] ?? [];
      const d = run.map((p, i) => `${i ? "L" : "M"}${Math.round(p.x * 10) / 10},${Math.round(p.y * 10) / 10}`).join("");
      const end = run[run.length - 1];
      return h(
        "div",
        { class: "kpi" },
        h("span", { class: "kpi-label" }, tile.label),
        h("b", { class: "kpi-value" }, tile.value),
        tile.delta?.text ? h("span", { class: `kpi-delta ${tile.delta.tone}` }, `${tile.delta.text} ${tile.against}`) : h("span", { class: "kpi-delta" }, tile.against ?? ""),
        run.length > 1
          ? s("svg", { class: "spark", viewBox: "0 0 100 24", "aria-hidden": "true" }, s("path", { class: "ln", d }), s("circle", { class: "dot", cx: end.x, cy: end.y, r: 2 }))
          : null,
      );
    }),
  );
}
