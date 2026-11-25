// Reports: what got done in a day, a week or thirty days, in words. A
// report is made when it is asked for, from the same history the Trends
// screen draws (so it needs no machine, and one for a week gone by can
// be made at any time), and can be copied or shared as text.
import { compact, dayLong, dayShort, deltaSaid, minutesSaid, windowTotals } from "../charts.js";
import { h } from "../ui.js";
import { wholeFrom } from "./trends.js";

const SPANS = [
  [1, "A day"],
  [7, "A week"],
  [30, "30 days"],
];
/** What was last chosen, while the app is open: how long, and the day it ends on (null is today). */
let chosen = 7;
let ending = null;

const plural = (n, one, many = `${one}s`) => `${compact(n)} ${n === 1 ? one : many}`;
/** "a, b and c". */
const listed = (items) => (items.length <= 1 ? items.join("") : `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`);
const middle = (values) => {
  const sorted = values.filter((v) => v !== null && v !== undefined).sort((a, b) => a - b);
  if (sorted.length === 0) return null;
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
};

/** The day `days` before or after a date. */
export function dayFrom(date, days) {
  return new Date(Date.parse(`${date}T00:00:00Z`) + days * 86_400_000).toISOString().slice(0, 10);
}

/** A trends document as a report: a title, what it covers, a headline,
 * and sections of sentences (some with the things themselves to open).
 * `counts` is the queue as it stands now, said only in a report that
 * ends today. `text` is the whole of it as plain text.
 * @param {any} doc
 * @param {{ claimed?: number, queued?: number, blocked?: number } | null} [counts] */
export function reportOf(doc, counts = null) {
  const n = Math.min(doc.window, doc.days.length);
  const first = doc.days.length - n;
  const cut = (values) => values.slice(first);
  const days = cut(doc.days);
  const end = days[days.length - 1];
  const today = end === doc.today;
  const before = n === 1 ? "the day before" : `the ${n} days before`;
  // A total is set against the days before only when those are whole.
  const firstWhole = doc.history.whole ? doc.history.from : doc.history.covered ? wholeFrom(doc.history.covered) : null;
  const whole = firstWhole !== null && firstWhole <= days[0];
  const beforeWhole = whole && doc.days.length >= n * 2 && firstWhole <= doc.days[doc.days.length - n * 2];
  const against = (values) => {
    const totals = windowTotals(values, n);
    const delta = beforeWhole ? deltaSaid(totals.now, totals.before) : { text: "" };
    return { n: totals.now, said: delta.text ? ` (${delta.text} on ${before})` : "" };
  };
  const sum = (values) => cut(values).reduce((total, v) => total + (v ?? 0), 0);

  const title = n === 1 ? (today ? "Today so far" : dayLong(end)) : `${n} days to ${dayLong(end)}`;
  const period = n === 1 ? dayLong(end) : `${dayLong(days[0])} to ${dayLong(end)}`;
  const merged = against(doc.merged.all);
  const kinds = [
    ["feat", "feature"],
    ["fix", "fix", "fixes"],
    ["docs", "of docs", "of docs"],
    ["test", "of tests", "of tests"],
    ["other", "other"],
  ]
    .map(([id, one, many]) => [sum(doc.merged.kinds[id] ?? []), one, many])
    .filter(([count]) => count > 0)
    .map(([count, one, many]) => plural(count, one, many));
  const featuresAll = doc.days.map((_, i) => doc.projects.reduce((total, p) => total + (doc.features.projects[p.id]?.[i] ?? 0), 0));
  const features = against(featuresAll);
  const headline = `${plural(merged.n, "pull request")} merged${merged.said}, ${compact(features.n)} of them features.`;

  const sections = [];
  const mergedLines = [`${plural(merged.n, "pull request")} merged${merged.said}.`];
  if (kinds.length > 0) mergedLines.push(`That is ${listed(kinds)}.`);
  const typical = middle(cut(doc.merged.minutes));
  if (typical !== null) mergedLines.push(`The middle one waited ${minutesSaid(typical)} from opened to merged.`);
  if (n > 1 && merged.n > 0) {
    const perDay = cut(doc.merged.all);
    const most = Math.max(...perDay);
    mergedLines.push(`The busiest day was ${dayLong(days[perDay.indexOf(most)])}, with ${compact(most)}.`);
  }
  sections.push({ title: "Merged", lines: mergedLines, links: [] });

  const projects = doc.projects
    .map((p) => ({ title: p.title, count: sum(doc.features.projects[p.id] ?? []), latest: doc.features.latest[p.id] ?? [] }))
    .filter((p) => p.count > 0)
    .sort((a, b) => b.count - a.count || (a.title < b.title ? -1 : 1));
  if (projects.length > 0) {
    sections.push({
      title: "Features, by project",
      lines: [`${plural(features.n, "feature")} merged${features.said}: ${listed(projects.map((p) => `${compact(p.count)} in ${p.title}`))}.`],
      // The latest three of each, to open.
      links: projects.flatMap((p) => p.latest.slice(0, 3).map((f) => ({ text: f.title, meta: `${p.title} · #${f.number}`, url: f.url }))),
    });
  }

  const opened = sum(doc.issues.opened);
  const closed = against(doc.issues.closed);
  const net = closed.n > opened ? `the backlog went down by ${compact(closed.n - opened)}` : opened > closed.n ? `the backlog grew by ${compact(opened - closed.n)}` : "the backlog stayed level";
  sections.push({
    title: "Issues",
    lines: [
      `${plural(opened, "issue")} opened and ${compact(closed.n)} closed${closed.said}: ${net}.`,
      `${plural(sum(doc.bugs.opened), "bug")} reported and ${compact(sum(doc.bugs.closed))} fixed.`,
      `${plural(sum(doc.prs.opened), "pull request")} opened; ${compact(sum(doc.prs.closed))} closed without merging.`,
    ],
    links: [],
  });

  if (!("error" in doc.tasks)) {
    const finished = against(doc.tasks.all);
    const byKind = doc.tasks.kinds
      .map((k) => [k.kind, sum(k.days)])
      .filter(([, count]) => count > 0)
      .sort((a, b) => (a[0] === "other" ? 1 : b[0] === "other" ? -1 : b[1] - a[1]))
      .map(([kind, count]) => `${compact(count)} ${kind === "other" ? "of other kinds" : kind}`);
    const lines = [`${plural(finished.n, "task")} finished${finished.said}${byKind.length > 0 ? `: ${listed(byKind)}` : ""}.`];
    if (today && counts) lines.push(`Right now ${compact(counts.claimed ?? 0)} are in progress, ${compact(counts.queued ?? 0)} queued and ${compact(counts.blocked ?? 0)} blocked.`);
    sections.push({ title: "The queue", lines, links: [] });
  }

  const notes = [];
  if (!whole) {
    notes.push(
      firstWhole === null
        ? "GitHub's history is still being read for the first time: these numbers are what has been read so far."
        : `GitHub's history is whole from ${dayLong(firstWhole)} on: days before that in this report are undercounted. It is still being read.`,
    );
  }
  if (today) notes.push(n === 1 ? "The day is not over." : "Today is not over.");
  if (doc.partial && doc.why) notes.push(doc.why);

  const text = [
    `${title} (${period}, UTC)`,
    "",
    ...sections.flatMap((s) => [s.title, ...s.lines, ...s.links.map((l) => `- ${l.text} (${l.meta}) ${l.url}`), ""]),
    ...notes,
  ]
    .join("\n")
    .trim();
  return { title, period, end, today, whole, headline, sections, notes, text };
}

async function view(sub, app) {
  const [span, end] = (sub || "").split("/");
  if (SPANS.some(([d]) => String(d) === span)) {
    chosen = Number(span);
    ending = /^\d{4}-\d{2}-\d{2}$/.test(end || "") ? end : null;
  }
  const doc = await app.api(app.freshly(`trends?days=${chosen}${ending ? `&end=${ending}` : ""}`));
  const report = reportOf(doc, app.counts());
  const to = (days, last) => `more/reports/${days}${last && last < doc.today ? `/${last}` : ""}`;
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(report.text);
      app.toast("Copied.");
    } catch {
      app.toast("This device would not let it be copied.", true);
    }
  };
  const share = navigator.share ? () => navigator.share({ title: report.title, text: report.text }).catch(() => {}) : null;
  const earliest = doc.history.from;
  return [
    h(
      "div",
      { class: "seg", role: "group", "aria-label": "How long a report" },
      SPANS.map(([d, label]) => h("button", { "aria-pressed": String(d === chosen), onclick: () => app.go(to(d, ending)) }, label)),
    ),
    h(
      "div",
      { class: "row" },
      h("button", { class: "ghost small", disabled: dayFrom(report.end, -chosen) < earliest, onclick: () => app.go(to(chosen, dayFrom(report.end, -chosen))) }, "‹ Earlier"),
      h("button", { class: "ghost small", disabled: report.today, onclick: () => app.go(to(chosen, dayFrom(report.end, chosen))) }, "Later ›"),
      h("span", { class: "grow" }),
      h("button", { class: "ghost small", onclick: copy }, "Copy"),
      share ? h("button", { class: "small", onclick: share }, "Share") : null,
    ),
    h("div", { class: "card hot" }, h("h3", {}, report.title), h("p", { class: "meta" }, `${report.period} · UTC`), h("p", {}, report.headline)),
    ...report.notes.map((text) => h("div", { class: "card flag" }, h("p", { class: "meta" }, text))),
    ...report.sections.map((s) =>
      h(
        "div",
        { class: "card" },
        h("h3", {}, s.title),
        s.lines.map((line) => h("p", {}, line)),
        s.links.map((l) => h("a", { class: "link", href: l.url, target: "_blank", rel: "noopener" }, l.text, h("span", {}, l.meta))),
      ),
    ),
    h("h2", {}, "Something these do not cover"),
    h("p", { class: "meta" }, "Ask for it in your own words: a question on Ask is answered by an agent, with what the answer rests on, and the answer stays there to read."),
    h("div", { class: "row" }, h("button", { class: "ghost", onclick: () => app.go("ask") }, "Ask a question"), h("button", { class: "ghost", onclick: () => app.go(`more/trends/${chosen === 1 ? 7 : chosen}`) }, "See it as charts")),
  ];
}

export default {
  route: "more/reports",
  title: "Reports",
  group: "Plan",
  says: "a day, a week or thirty days in words, to read, copy or share",
  view,
};
