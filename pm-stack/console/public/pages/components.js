// Components: every shared element the app is built with, shown once
// each on one screen (#14433). Change the theme, the text size or the
// spacing (More → Appearance) and this screen shows what every element
// does under it, so a look can be judged on all of them at a glance —
// and a new screen has something to copy instead of inventing its own.
//
// The census below is the list of shared elements in use. ELEMENTS is
// the machine-readable form the tests hold this page to: every entry
// is drawn here once, with the class app.css gives it, built with `h`
// from ../ui.js or the chart kit in ../charts.js. Two entries are the
// app frame's own (toast, sheet): they are listed so the census is
// whole, and said to be the frame's rather than drawn out of place.
import { lineChart, stackedColumns, statTiles } from "../charts.js";
import { h } from "../ui.js";

/** The shared elements in use: the name a screen asks for, the class
 * (or kit function) that draws it, and where its home is. */
export const ELEMENTS = [
  { name: "card", made: "class .card", home: "app.css" },
  { name: "chip", made: "class .chip", home: "app.css" },
  { name: "row", made: "class .row", home: "app.css" },
  { name: "tile", made: "class .tile", home: "app.css" },
  { name: "link", made: "class .link", home: "app.css" },
  { name: "option", made: "class .option", home: "app.css" },
  { name: "seg", made: "class .seg", home: "app.css" },
  { name: "stat", made: "class .stat", home: "app.css" },
  { name: "fold", made: "class .card.fold (details)", home: "app.css" },
  { name: "empty", made: "class .empty", home: "app.css" },
  { name: "stat tiles", made: "statTiles()", home: "charts.js" },
  { name: "stacked columns", made: "stackedColumns()", home: "charts.js" },
  { name: "line chart", made: "lineChart()", home: "charts.js" },
  { name: "toast", made: "class .toast", home: "app frame (app.js)" },
  { name: "sheet", made: "dialog element", home: "app frame (app.js)" },
];

function view() {
  return [
    h("p", { class: "meta" }, "Every shared element, once. Judge a theme, a text size or a spacing on all of them here."),
    h("h2", {}, "Card, fold and empty"),
    h("div", { class: "card" }, h("p", {}, "A card: the plain box most things live in.")),
    h("div", { class: "card flag" }, h("p", { class: "meta" }, "A flagged card: something to notice.")),
    h(
      "details",
      { class: "card fold", open: true },
      h("summary", {}, h("b", {}, "A fold"), " opened"),
      h("p", { class: "meta" }, "A card that opens and closes."),
    ),
    h("div", { class: "empty" }, "An empty state: what a screen says when there is nothing."),
    h("h2", {}, "Chip, row, stat and tile"),
    h(
      "div",
      { class: "row" },
      h("span", { class: "chip" }, "chip"),
      h("span", { class: "chip good" }, "good"),
      h("span", { class: "chip warn" }, "warn"),
      h("span", { class: "chip accent" }, "accent"),
    ),
    h("div", { class: "stats" }, h("button", { class: "stat", type: "button" }, h("b", {}, "42"), h("span", {}, "a stat"))),
    h("button", { class: "tile", type: "button" }, "A tile", h("span", {}, "what a tile says under its name")),
    h("h2", {}, "Link, option and seg"),
    h("a", { class: "link", href: "#/more/components" }, "A link", h("span", {}, "what a link says under its name")),
    h(
      "div",
      { class: "options" },
      h("button", { class: "option", type: "button", "aria-pressed": "true" }, "An option, chosen"),
      h("button", { class: "option", type: "button", "aria-pressed": "false" }, "An option"),
    ),
    h(
      "div",
      { class: "seg", role: "group", "aria-label": "An example segmented control" },
      h("button", { type: "button", "aria-pressed": "true" }, "Chosen"),
      h("button", { type: "button", "aria-pressed": "false" }, "Other"),
    ),
    h("h2", {}, "The chart kit"),
    statTiles([{ label: "A stat tile", value: "7", delta: { text: "▲ 10%", tone: "good" }, against: "on the days before", spark: [1, 2, 3, 2, 4] }]),
    stackedColumns({
      title: "Stacked columns",
      note: "From the chart kit, with sample numbers.",
      labels: ["One", "Two"],
      long: ["One", "Two"],
      series: [{ name: "sample", slot: 1, values: [2, 3] }],
      unit: "drawn",
    }),
    lineChart({
      title: "A line chart",
      labels: ["One", "Two"],
      long: ["One", "Two"],
      series: [{ name: "sample", slot: 1, values: [1, 3], total: "3" }],
    }),
    h("h2", {}, "The app frame's own"),
    h(
      "div",
      { class: "card" },
      h("p", { class: "meta" }, "Toast (class .toast) and sheet (a dialog) belong to the app frame in app.js, not to a screen, so they are listed here and not drawn out of place."),
    ),
  ];
}

export default {
  route: "more/components",
  title: "Components",
  group: "Operations",
  says: "every shared element once, to judge a theme or spacing on all of them",
  view,
};
