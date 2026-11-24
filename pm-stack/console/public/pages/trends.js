// Trends: what the fleet got done, day by day. Pull requests merged and
// what they were, features by project with the features themselves,
// issues opened against issues closed, how long a pull request waits,
// and tasks finished. Read by whoever serves the app from GitHub and the
// queue's store (src/trends.ts); no machine is needed for any of it.
import { compact, dayLong, dayShort, deltaSaid, groupsOf, lineChart, minutesSaid, rollUp, stackedColumns, statTiles, windowTotals } from "../charts.js";
import { h } from "../ui.js";

const WINDOWS = [
  [7, "7 days"],
  [14, "14 days"],
  [30, "30 days"],
  [90, "90 days"],
];
/** The window last chosen, while the app is open. */
let chosen = 30;

/** Which colour each kind of pull request and each project is drawn in.
 * Fixed: a kind or a project is the same colour in every window. */
const KIND_SERIES = [
  ["feat", "features", 1],
  ["fix", "fixes", 2],
  ["docs", "docs", 3],
  ["test", "tests", 4],
  ["other", "everything else", "other"],
];
const PROJECT_SLOTS = { id: 1, fleet: 2, atlas: 3, "pm-stack": 4, mcpx: 5, other: "other" };

const middle = (values) => {
  const sorted = values.filter((v) => v !== null && v !== undefined).sort((a, b) => a - b);
  if (sorted.length === 0) return null;
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
};

/** The document as the screen draws it: the window's days (the same
 * number before them are only for setting a total against), by the day
 * up to a month and by the week beyond, with each chart's series. */
export function shape(doc) {
  const n = Math.min(doc.window, doc.days.length);
  const first = doc.days.length - n;
  const cut = (values) => values.slice(first);
  const days = cut(doc.days);
  const weekly = n > 31;
  const groups = groupsOf(n, weekly ? 7 : 1);
  const columns = (values) => rollUp(cut(values), groups);
  const labels = groups.map(([, to]) => dayShort(days[to - 1]));
  const long = groups.map(([from, to]) => (weekly ? `${dayShort(days[from])} to ${dayShort(days[to - 1])}` : dayLong(days[from])));
  const against = `on the ${n} days before`;
  const kpi = (label, values) => {
    const totals = windowTotals(values, n);
    return { label, value: compact(totals.now), delta: deltaSaid(totals.now, totals.before), against: totals.before === null ? `in ${n} days` : against, spark: cut(values) };
  };
  const featuresAll = doc.days.map((_, i) => doc.projects.reduce((sum, p) => sum + (doc.features.projects[p.id]?.[i] ?? 0), 0));
  const typical = middle(cut(doc.merged.minutes));
  const tasks = "error" in doc.tasks ? null : doc.tasks;
  const kpis = [kpi("Pull requests merged", doc.merged.all), kpi("Features merged", featuresAll), kpi("Issues closed", doc.issues.closed)];
  if (tasks) kpis.push(kpi("Tasks finished", tasks.all));
  kpis.push({ label: "Open to merged, typically", value: minutesSaid(typical), against: "the middle pull request", spark: cut(doc.merged.minutes).map((v) => v ?? 0) });
  const taskKinds = tasks ? [...tasks.kinds].sort((a, b) => (a.kind === "other" ? 1 : b.kind === "other" ? -1 : a.kind < b.kind ? -1 : 1)) : [];
  let slot = 0;
  const sum = (values) => cut(values).reduce((total, v) => total + (v ?? 0), 0);
  return {
    n,
    days,
    weekly,
    labels,
    long,
    dayLabels: days.map(dayShort),
    dayLong: days.map(dayLong),
    kpis,
    mergedByKind: KIND_SERIES.map(([id, name, s]) => ({ name, slot: s, values: columns(doc.merged.kinds[id] ?? []) })),
    featuresByProject: doc.projects.map((p) => ({ name: p.title, slot: PROJECT_SLOTS[p.id] ?? "other", values: columns(doc.features.projects[p.id] ?? []) })),
    issues: [
      { name: "opened", slot: 2, values: cut(doc.issues.opened), total: compact(sum(doc.issues.opened)) },
      { name: "closed", slot: 1, values: cut(doc.issues.closed), total: compact(sum(doc.issues.closed)) },
    ],
    bugs: [
      { name: "reported", slot: 2, values: cut(doc.bugs.opened), total: compact(sum(doc.bugs.opened)) },
      { name: "fixed", slot: 1, values: cut(doc.bugs.closed), total: compact(sum(doc.bugs.closed)) },
    ],
    minutes: [{ name: "open to merged", slot: 1, values: cut(doc.merged.minutes), total: minutesSaid(typical) }],
    tasks: tasks ? taskKinds.map((k) => ({ name: k.kind, slot: k.kind === "other" ? "other" : ++slot, values: columns(k.days) })) : null,
    tasksSince: tasks?.since ?? null,
    // Each project that merged a feature in the window, the most first.
    features: doc.projects
      .map((p) => ({ id: p.id, title: p.title, slot: PROJECT_SLOTS[p.id] ?? "other", count: sum(doc.features.projects[p.id] ?? []), latest: doc.features.latest[p.id] ?? [] }))
      .filter((p) => p.count > 0)
      .sort((a, b) => b.count - a.count || (a.title < b.title ? -1 : 1)),
  };
}

/** What has to be said before the charts: history still being read,
 * something that could not be read, and whose days these are. */
export function notes(doc) {
  const out = [];
  if (!doc.history.whole) {
    out.push(`The history is still being read from GitHub. It has reached ${dayLong(doc.history.readTo.slice(0, 10))}: counts after that are not all in yet. It reads on by itself; ↻ reads on now.`);
  }
  if (doc.partial && doc.why) out.push(doc.why);
  return out;
}

async function view(sub, app) {
  const days = WINDOWS.some(([d]) => String(d) === sub) ? Number(sub) : chosen;
  chosen = days;
  const doc = await app.api(app.freshly(`trends?days=${days}`));
  const sh = shape(doc);
  const per = sh.weekly ? "by week" : "by day";
  const today = sh.weekly ? "This week is still going." : "Today is still going.";
  const out = [
    h(
      "div",
      { class: "seg", role: "group", "aria-label": "How far back" },
      WINDOWS.map(([d, label]) => h("button", { "aria-pressed": String(d === days), onclick: () => app.go(`more/trends/${d}`) }, label)),
    ),
    ...notes(doc).map((text) => h("div", { class: "card flag" }, h("p", { class: "meta" }, text))),
    statTiles(sh.kpis),
    stackedColumns({ title: `Pull requests merged, ${per}`, note: `By what the title says each is. ${today}`, labels: sh.labels, long: sh.long, series: sh.mergedByKind, unit: "merged", partial: true }),
    stackedColumns({ title: `Features merged, ${per}, by project`, note: "A pull request whose title begins feat, under the project its scope belongs to.", labels: sh.labels, long: sh.long, series: sh.featuresByProject, unit: "features", partial: true }),
  ];
  if (sh.features.length > 0) {
    out.push(h("h2", {}, `Features merged in ${sh.n} days, by project`));
    for (const project of sh.features) {
      out.push(
        h(
          "details",
          { class: "card fold", open: project === sh.features[0] },
          h("summary", {}, h("i", { class: `sw s${project.slot === "other" ? "o" : project.slot}` }), h("b", {}, project.title), ` ${compact(project.count)}`),
          project.latest.map((f) => h("a", { class: "link", href: f.url, target: "_blank", rel: "noopener" }, f.title, h("span", {}, `#${f.number} · merged ${app.ago(f.at)}`))),
          project.count > project.latest.length ? h("p", { class: "meta" }, `The latest ${project.latest.length} of ${compact(project.count)}.`) : null,
        ),
      );
    }
  }
  out.push(
    lineChart({ title: "Issues opened and closed, by day", note: "Closed above opened is the backlog going down.", labels: sh.dayLabels, long: sh.dayLong, series: sh.issues }),
    lineChart({ title: "Bugs reported and fixed, by day", labels: sh.dayLabels, long: sh.dayLong, series: sh.bugs }),
    lineChart({ title: "From opened to merged, by day", note: "The middle pull request of each day: half waited less, half more.", labels: sh.dayLabels, long: sh.dayLong, series: sh.minutes, format: minutesSaid, unit: "typically" }),
  );
  if (sh.tasks) {
    out.push(
      stackedColumns({
        title: `Tasks finished, ${per}`,
        note: sh.tasksSince && sh.tasksSince > sh.days[0] ? `By kind. The queue has them from ${dayLong(sh.tasksSince)}. ${today}` : `By kind. ${today}`,
        labels: sh.labels,
        long: sh.long,
        series: sh.tasks,
        unit: "finished",
        partial: true,
      }),
    );
  }
  out.push(h("p", { class: "meta" }, `Days are UTC days.${doc.history.readAt ? ` GitHub's history was last read ${app.ago(doc.history.readAt)}.` : ""}`));
  return out;
}

export default {
  route: "more/trends",
  title: "Trends",
  group: "Plan",
  says: "merged, features by project, issues and tasks, day by day",
  view,
};
