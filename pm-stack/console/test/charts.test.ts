// The charts: their sums, and what they draw.
//
// Oracles: every number is worked out by hand in the comment beside it.
// What is drawn is read back from a stand-in page (test/dom.ts) that
// remembers elements, attributes and listeners; the rules it is held to
// are the ones written at the top of public/charts.js. That a browser
// draws it as meant was looked at in Chromium on the laptop, in a dark
// theme and a light one (see the design record).
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  compact,
  dayLong,
  dayShort,
  deltaSaid,
  groupsOf,
  lineChart,
  lineRuns,
  minutesSaid,
  niceMax,
  placeAt,
  rollUp,
  stackedColumns,
  stackGeometry,
  statTiles,
  windowTotals,
} from "../public/charts.js";
import { type El, page } from "./dom.ts";

test("the top of a scale is the next clean number", () => {
  for (const [value, top] of [
    [0, 1],
    [-3, 1],
    [1, 1],
    [3, 5],
    [7, 10],
    [12, 20],
    [23, 25],
    [26, 50],
    [99, 100],
    [100, 100],
    [101, 200],
    [250, 250],
    [0.4, 0.5],
    [1288, 2000],
  ]) {
    expect([value, niceMax(value)]).toEqual([value, top]);
  }
});

test("numbers, minutes and days as they are written", () => {
  expect([0, 999, 1284, 9999, 12_940, 4_200_000, 2.5].map(compact)).toEqual([
    "0",
    "999",
    "1,284",
    "9,999",
    "12.9K",
    "4.2M",
    "2.5",
  ]);
  expect([compact(null), compact(undefined), compact(Number.NaN)]).toEqual(["–", "–", "–"]);
  // 90 minutes is an hour and a half; 36 hours is a day and a half.
  expect([45, 89, 90, 210, 2159, 2160, 14_400].map(minutesSaid)).toEqual([
    "45 min",
    "89 min",
    "1.5 h",
    "3.5 h",
    "36 h",
    "1.5 d",
    "10 d",
  ]);
  expect(minutesSaid(null)).toBe("–");
  // The 24th of November 2026 is a Tuesday; the 1st, a Sunday.
  expect([dayShort("2026-11-24"), dayLong("2026-11-24"), dayLong("2026-11-01"), dayShort("2026-01-05")]).toEqual([
    "24 Nov",
    "Tue 24 Nov",
    "Sun 1 Nov",
    "5 Jan",
  ]);
});

test("days in groups counted back from the newest, and a series added up by them", () => {
  expect(groupsOf(14, 7)).toEqual([
    [0, 7],
    [7, 14],
  ]);
  // Ten days in sevens: the newest seven are whole, the oldest three are what is left.
  expect(groupsOf(10, 7)).toEqual([
    [0, 3],
    [3, 10],
  ]);
  expect(groupsOf(3, 1)).toEqual([
    [0, 1],
    [1, 2],
    [2, 3],
  ]);
  expect(groupsOf(0, 7)).toEqual([]);
  expect(
    rollUp(
      [1, 2, 3, 4, 5],
      [
        [0, 2],
        [2, 5],
      ],
    ),
  ).toEqual([3, 12]);
  expect(rollUp([1, null, 3], [[0, 3]])).toEqual([4]);
});

test("a total and the one before it, and how one stands against the other", () => {
  expect(windowTotals([1, 2, 3, 4], 2)).toEqual({ now: 7, before: 3 });
  // Three days cannot hold two before the last two.
  expect(windowTotals([1, 2, 3], 2)).toEqual({ now: 5, before: null });
  expect(windowTotals([], 7)).toEqual({ now: 0, before: null });
  expect(deltaSaid(110, 100)).toEqual({ text: "▲ 10%", tone: "up" });
  expect(deltaSaid(90, 100)).toEqual({ text: "▼ 10%", tone: "down" });
  expect(deltaSaid(100, 100)).toEqual({ text: "level", tone: "" });
  // Under half a percent either way is level.
  expect(deltaSaid(1004, 1000)).toEqual({ text: "level", tone: "" });
  expect(deltaSaid(5, 0)).toEqual({ text: "▲ from none", tone: "up" });
  expect(deltaSaid(0, 0)).toEqual({ text: "level", tone: "" });
  expect(deltaSaid(5, null)).toEqual({ text: "", tone: "" });
  expect(deltaSaid(0, 40)).toEqual({ text: "▼ 100%", tone: "down" });
});

test("where the columns of a stack go, and how tall their parts are", () => {
  const plot = { x: 0, y: 0, w: 100, h: 100 };
  // Two columns in 100: a band of 50 each; a column is 24 at most, so 13 in from its band's edge.
  const g = stackGeometry(
    [
      [10, 0],
      [10, 20],
    ],
    20,
    plot,
  );
  expect(g.band).toBe(50);
  expect(g.columns.map((c) => [c.x, c.w, c.total])).toEqual([
    [13, 24, 20],
    [63, 24, 20],
  ]);
  // First column: the bottom part is half the height (50) less the gap of 2 above it; the top part is its whole 50.
  expect(g.columns[0].parts).toEqual([
    { series: 0, value: 10, y: 52, h: 48, top: false },
    { series: 1, value: 10, y: 0, h: 50, top: true },
  ]);
  // Second column: one part, the whole height, and it is the top.
  expect(g.columns[1].parts).toEqual([{ series: 1, value: 20, y: 0, h: 100, top: true }]);
  // Something small beside something large is still there to see: at least 1 tall.
  const small = stackGeometry([[1], [999]], 1000, plot).columns[0].parts[0];
  expect([small.h, small.y]).toEqual([1, 99]);
  // Thirty columns in 320: each band 10.67, each column 8.67 (a gap of 2 between neighbours).
  const thirty = stackGeometry([Array(30).fill(1)], 1, { x: 34, y: 10, w: 320, h: 128 });
  expect(Math.round(thirty.columns[0].w * 100) / 100).toBe(8.67);
  expect(Math.round((thirty.columns[1].x - thirty.columns[0].x - thirty.columns[0].w) * 100) / 100).toBe(2);
  // Nothing to draw is no columns, and a column of nothing has no parts.
  expect(stackGeometry([], 1, plot).columns).toEqual([]);
  expect(stackGeometry([[0]], 1, plot).columns[0].parts).toEqual([]);
});

test("the points of a line, broken where a day has no number", () => {
  const plot = { x: 0, y: 0, w: 90, h: 100 };
  expect(lineRuns([0, 10, null, 5], 10, plot)).toEqual([
    [
      { i: 0, x: 0, y: 100 },
      { i: 1, x: 30, y: 0 },
    ],
    [{ i: 3, x: 90, y: 50 }],
  ]);
  // One day sits in the middle; a number over the top of the scale is drawn at the top.
  expect(lineRuns([5], 10, plot)).toEqual([[{ i: 0, x: 45, y: 50 }]]);
  expect(lineRuns([20], 10, plot)[0][0].y).toBe(0);
  expect(lineRuns([null, null], 10, plot)).toEqual([]);
});

test("which day a pointer is over", () => {
  // The drawing is 360 across and its plot runs from 34 for 320. Half way across is 180: 146 into the plot.
  // Ten columns of 32: 146 is in the fifth (place 4).
  expect(placeAt(0.5, 10)).toBe(4);
  // Ten points 35.6 apart: 146 is nearest the fifth (142).
  expect(placeAt(0.5, 10, undefined, false)).toBe(4);
  // Off either end is the first or the last, and no days is no place.
  expect([placeAt(0, 10), placeAt(1, 10), placeAt(-1, 10), placeAt(2, 10)]).toEqual([0, 9, 0, 9]);
  expect(placeAt(0.5, 0)).toBe(-1);
  expect(placeAt(0.5, 1, undefined, false)).toBe(0);
});

const text = (el: El) => el.textContent.replace(/\s+/g, " ").trim();

test("stacked columns: a column a day, today fainter, a legend with totals, the numbers as a table, and each day said when chosen", () => {
  page();
  const chart = stackedColumns({
    title: "Pull requests merged, by day",
    note: "By what the title says each is.",
    labels: ["22 Nov", "23 Nov", "24 Nov"],
    long: ["Sun 22 Nov", "Mon 23 Nov", "Tue 24 Nov"],
    series: [
      { name: "features", slot: 1, values: [3, 0, 2] },
      { name: "fixes", slot: 2, values: [1, 4, 0] },
      { name: "docs", slot: 3, values: [0, 0, 0] },
      { name: "everything else", slot: "other", values: [0, 1, 1] },
    ],
    unit: "merged",
    partial: true,
  }) as unknown as El;
  expect(chart.tag).toBe("figure");
  expect(text(chart.find("h3")[0])).toBe("Pull requests merged, by day");
  const columns = chart.find(".col");
  expect(columns.length).toBe(3);
  expect(columns.map((c) => c.classList.contains("partial"))).toEqual([false, false, true]);
  // Each part is drawn in its series' class and no other colour; the top of each column is the rounded one.
  expect(columns[0].kids.map((k) => [(k as El).tag, (k as El).className])).toEqual([
    ["rect", "s1"],
    ["path", "s2"],
  ]);
  expect(columns[2].kids.map((k) => [(k as El).tag, (k as El).className])).toEqual([
    ["rect", "s1"],
    ["path", "so"],
  ]);
  // The scale: the tallest day is 5, so it runs 0, 2.5, 5.
  expect(chart.find(".tick").map(text).slice(0, 3)).toEqual(["0", "2.5", "5"]);
  // The series with nothing in it (docs) is not drawn, named or coloured; the others keep their own colours.
  const legend = chart.find(".legend")[0];
  expect(legend.find("li").map(text)).toEqual(["features 5", "fixes 5", "everything else 2"]);
  expect(legend.find(".sw").map((sw) => sw.className)).toEqual(["sw s1", "sw s2", "sw so"]);
  // At rest the line under the title says the whole.
  const readout = chart.find(".readout")[0];
  expect(text(readout)).toBe("12 merged");
  // The table has every number, and the total.
  const rows = chart
    .find("tbody")[0]
    .find("tr")
    .map((tr) => tr.kids.map((k) => text(k as El)));
  expect(chart.find("thead")[0].find("th").map(text)).toEqual(["When", "features", "fixes", "everything else", "All"]);
  expect(rows).toEqual([
    ["Sun 22 Nov", "3", "1", "0", "4"],
    ["Mon 23 Nov", "0", "4", "1", "5"],
    ["Tue 24 Nov", "2", "0", "1", "3"],
  ]);
  // Arrowing to a day says it: the number first, every series at once. Today is "so far".
  const svg = chart.find("svg")[0];
  expect(svg.attrs.tabindex).toBe("0");
  expect(svg.attrs["aria-label"]).toBe(
    "Pull requests merged, by day. 12 merged in all. The arrow keys read each column.",
  );
  expect(svg.fire("keydown", { key: "ArrowRight" })).toBe(true);
  expect(text(readout)).toBe("Sun 22 Nov · 4 3 features 1 fixes 0 everything else");
  expect(columns.map((c) => c.classList.contains("on"))).toEqual([true, false, false]);
  expect(svg.classList.contains("choosing")).toBe(true);
  svg.fire("keydown", { key: "End" });
  expect(text(readout)).toBe("Tue 24 Nov · 3 so far 2 features 0 fixes 1 everything else");
  svg.fire("keydown", { key: "ArrowRight" }); // already the last
  expect(columns.map((c) => c.classList.contains("on"))).toEqual([false, false, true]);
  svg.fire("keydown", { key: "ArrowLeft" });
  expect(text(readout).startsWith("Mon 23 Nov · 5")).toBe(true);
  // A pointer two thirds across is over the second column of three; a finger lifted leaves it; a mouse moved off lets go.
  svg.fire("pointermove", { clientX: 200 });
  expect(text(readout).startsWith("Mon 23 Nov")).toBe(true);
  svg.fire("pointermove", { clientX: 40 });
  expect(text(readout).startsWith("Sun 22 Nov")).toBe(true);
  svg.fire("pointerleave", { pointerType: "touch" });
  expect(text(readout).startsWith("Sun 22 Nov")).toBe(true);
  svg.fire("pointerleave", { pointerType: "mouse" });
  expect(text(readout)).toBe("12 merged");
  expect(svg.classList.contains("choosing")).toBe(false);
  // Another key is not taken from the page.
  expect(svg.fire("keydown", { key: "Tab" })).toBe(false);
  svg.fire("keydown", { key: "Home" });
  svg.fire("keydown", { key: "Escape" });
  expect(text(readout)).toBe("12 merged");
});

test("one series needs no legend, and a chart of nothing still has its frame", () => {
  page();
  const one = stackedColumns({
    title: "Tasks",
    labels: ["a", "b"],
    long: ["A", "B"],
    series: [{ name: "finished", slot: 1, values: [2, 3] }],
    unit: "finished",
  }) as unknown as El;
  expect(one.find(".legend")).toEqual([]);
  expect(one.find("thead")[0].find("th").map(text)).toEqual(["When", "finished"]);
  one.find("svg")[0].fire("keydown", { key: "Home" });
  expect(text(one.find(".readout")[0])).toBe("A · 2");
  const none = stackedColumns({
    title: "Nothing yet",
    labels: ["a"],
    long: ["A"],
    series: [
      { name: "x", slot: 1, values: [0] },
      { name: "y", slot: 2, values: [0] },
    ],
    unit: "merged",
  }) as unknown as El;
  expect(text(none.find(".readout")[0])).toBe("0 merged");
  expect(none.find(".col").length).toBe(1);
  expect(none.find(".tick").map(text).slice(0, 3)).toEqual(["0", "0.5", "1"]);
});

test("lines: a legend for two, a wash and no legend for one, a dot that follows the day chosen, and gaps left as gaps", () => {
  page();
  const two = lineChart({
    title: "Issues opened and closed, by day",
    labels: ["22 Nov", "23 Nov", "24 Nov"],
    long: ["Sun 22 Nov", "Mon 23 Nov", "Tue 24 Nov"],
    series: [
      { name: "opened", slot: 2, values: [4, 2, 6], total: "12" },
      { name: "closed", slot: 1, values: [1, 5, 3], total: "9" },
    ],
  }) as unknown as El;
  expect(two.find(".legend")[0].find("li").map(text)).toEqual(["opened 12", "closed 9"]);
  expect(
    two
      .find(".legend")[0]
      .find(".sw")
      .map((sw) => sw.className),
  ).toEqual(["sw ln s2", "sw ln s1"]);
  expect(two.find(".wash")).toEqual([]);
  expect(two.find("path").map((p) => p.className)).toEqual(["ln s2", "ln s1"]);
  const readout = two.find(".readout")[0];
  expect(text(readout)).toBe("3 days 12 opened 9 closed");
  const svg = two.find("svg")[0];
  // Each line ends on a dot ringed in the card's colour, on its last number: the scale tops out at 10,
  // so 6 is 60% up a plot 128 tall from 138: y 61.2; 3 is at 99.6. Both at the right edge, 354.
  const dots = two.find(".ring");
  expect(dots.map((d) => [d.attrs.cx, d.attrs.cy])).toEqual([
    ["354", "61.2"],
    ["354", "99.6"],
  ]);
  svg.fire("keydown", { key: "Home" });
  expect(text(readout)).toBe("Sun 22 Nov 4 opened 1 closed");
  // The dots moved to the first day, at the plot's left edge, and the hairline with them.
  expect(dots.map((d) => d.attrs.cx)).toEqual(["34", "34"]);
  expect(two.find(".hair")[0].attrs.x1).toBe("34");
  svg.fire("blur");
  expect(text(readout)).toBe("3 days 12 opened 9 closed");
  expect(dots.map((d) => d.attrs.cx)).toEqual(["354", "354"]);
  expect(Number(two.find(".hair")[0].attrs.x1)).toBeLessThan(0);

  const one = lineChart({
    title: "From opened to merged, by day",
    labels: ["a", "b", "c", "d"],
    long: ["A", "B", "C", "D"],
    series: [{ name: "open to merged", slot: 1, values: [30, null, 90, 120], total: "1.5 h" }],
    format: minutesSaid,
    unit: "typically",
  }) as unknown as El;
  expect(one.find(".legend")).toEqual([]);
  // The first day stands alone (a dot); the last two are a line with a wash under it.
  expect(one.find(".wash").length).toBe(1);
  expect(one.all((el) => el.tag === "circle" && !el.classList.contains("ring")).length).toBe(1);
  expect(text(one.find(".readout")[0])).toBe("4 days 1.5 h typically");
  one.find("svg")[0].fire("keydown", { key: "ArrowRight" });
  expect(text(one.find(".readout")[0])).toBe("A 30 min typically");
  one.find("svg")[0].fire("keydown", { key: "ArrowRight" });
  // A day with no number says so, and the dot leaves the plot.
  expect(text(one.find(".readout")[0])).toBe("B – typically");
  expect(Number(one.find(".ring")[0].attrs.cx)).toBeLessThan(0);
  // The scale is written the way the numbers are.
  expect(one.find(".tick").map(text).slice(0, 3)).toEqual(["0 min", "1.7 h", "3.3 h"]);
  // The table too.
  expect(one.find("tbody")[0].find("td").map(text)).toEqual(["30 min", "–", "1.5 h", "2 h"]);
});

test("the leading numbers: what each is, the number, how it stands, and the shape of its days", () => {
  page();
  const tiles = statTiles([
    {
      label: "Pull requests merged",
      value: "2,930",
      delta: { text: "▲ 12%", tone: "up" },
      against: "on the 7 days before",
      spark: [1, 3, 2, 5],
    },
    {
      label: "Issues closed",
      value: "40",
      delta: { text: "▼ 8%", tone: "down" },
      against: "on the 7 days before",
      spark: [2, 1],
    },
    { label: "Tasks finished", value: "7", delta: { text: "", tone: "" }, against: "in 3 days", spark: [7] },
  ]) as unknown as El;
  const each = tiles.find(".kpi");
  expect(each.map((t) => text(t))).toEqual([
    "Pull requests merged2,930▲ 12% on the 7 days before",
    "Issues closed40▼ 8% on the 7 days before",
    "Tasks finished7in 3 days",
  ]);
  // Up and down are said by the arrow and the figure; the class only adds the colour.
  expect(each.map((t) => t.find(".kpi-delta")[0].className)).toEqual(["kpi-delta up", "kpi-delta down", "kpi-delta"]);
  // A shape needs two days.
  expect(each.map((t) => t.find("svg").length)).toEqual([1, 1, 0]);
});

test("a chart writes no colour and no style of its own: marks take a class, and the classes take the theme's series", () => {
  const source = readFileSync(new URL("../public/charts.js", import.meta.url), "utf8");
  expect(source).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
  expect(source).not.toMatch(/\.style\b|"style"|\bfill: |\bstroke: /);
  const css = readFileSync(new URL("../public/app.css", import.meta.url), "utf8");
  const themes = readFileSync(new URL("../public/themes.css", import.meta.url), "utf8");
  for (let n = 1; n <= 6; n++) {
    expect(css).toContain(`.s${n} {\n  fill: var(--series-${n});`);
    expect(css).toContain(`.ln.s${n} {\n  stroke: var(--series-${n});`);
    expect(css).toContain(`.sw.s${n} {\n  background: var(--series-${n});`);
  }
  // "Everything else" is grey, in the theme's own.
  expect(css).toContain(".so {\n  fill: var(--quiet);");
  // The series colours: six, in the order they were checked in (blue, orange, aqua, yellow, magenta,
  // green: neighbours told apart under every kind of colour vision), stepped for a dark page and a
  // light one. The app's own :root has the same two sets as the themes' sheet.
  const dark = ["#3987e5", "#d95926", "#199e70", "#c98500", "#d55181", "#0ca30c"];
  const light = ["#2a78d6", "#eb6834", "#1baf7a", "#eda100", "#e87ba4", "#008300"];
  const set = (text: string) => [...text.matchAll(/--series-(\d): (#[0-9a-f]{6});/g)].map((m) => m[2]);
  const block = (text: string, from: string) => text.slice(text.indexOf(from), text.indexOf("}", text.indexOf(from)));
  expect(set(block(themes, '[data-scheme="dark"]'))).toEqual(dark);
  expect(set(block(themes, '[data-scheme="light"]'))).toEqual(light);
  expect(set(block(css, ":root {"))).toEqual(dark);
  expect(set(block(css.slice(css.indexOf("@media (prefers-color-scheme: light)")), ":root {"))).toEqual(light);
  // They are not the status colours of any theme's... of the app's own two, at least.
  for (const status of ["--good: #3fb950", "--warn: #d29922", "--bad: #f85149"]) expect(css).toContain(status);
  expect(dark).not.toContain("#3fb950");
});
