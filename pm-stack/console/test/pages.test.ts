// The app's pages that are modules of their own (public/pages), and the
// Trends page's sums.
//
// Oracles: that a page is whole and wired in is each file's own text and
// what the modules export, held against the three places a page has to
// be (its file, the list of pages, the list of the app's files). The
// Trends page's numbers are worked out by hand from a small document
// written out here.
import { expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import "../public/look.js";
import { routesKnown } from "../public/app.js";
import { PAGES } from "../public/pages/index.js";
import trends, { notes, shape, wholeFrom } from "../public/pages/trends.js";

const PUBLIC = join(import.meta.dir, "../public");

test("every page in the folder is on the list of pages, has what a page needs, and is kept for when there is no connection", () => {
  const files = readdirSync(join(PUBLIC, "pages"))
    .filter((name) => name.endsWith(".js") && name !== "index.js")
    .sort();
  expect(files.length).toBeGreaterThan(0);
  const listed = (
    JSON.parse(readFileSync(join(PUBLIC, "files.json"), "utf8")) as { files: { path: string; kept: string }[] }
  ).files;
  const index = readFileSync(join(PUBLIC, "pages/index.js"), "utf8");
  for (const file of files) {
    const name = file.replace(/\.js$/, "");
    // Imported by the list of pages, and in it.
    expect([file, index.includes(`from "./${file}";`)]).toEqual([file, true]);
    const page = PAGES.find((p: { route: string }) => p.route === `more/${name}`);
    expect([file, page !== undefined]).toEqual([file, true]);
    // Named in the app's files, and kept from the start (the app's script imports it).
    expect([file, listed.find((f) => f.path === `pages/${file}`)?.kept]).toEqual([file, "installed"]);
  }
  expect(PAGES.length).toBe(files.length);
  for (const page of PAGES as { route: string; title: string; group: string; says: string; view: unknown }[]) {
    expect([page.route, /^more\/[a-z][a-z-]*$/.test(page.route)]).toEqual([page.route, true]);
    expect([page.route, typeof page.title, page.title.length > 0]).toEqual([page.route, "string", true]);
    expect([page.route, ["Plan", "Operations"].includes(page.group)]).toEqual([page.route, true]);
    expect([page.route, typeof page.says === "string" && page.says.length > 10]).toEqual([page.route, true]);
    expect([page.route, typeof page.view]).toEqual([page.route, "function"]);
  }
  // The app gives each its address, and no two pages (or a page and a screen of the app's own) share one.
  const routes = routesKnown();
  for (const page of PAGES as { route: string }[])
    expect(routes.filter((r: string) => r === page.route).length).toBe(1);
  expect(new Set(routes).size).toBe(routes.length);
  // And its tile on More comes from the page itself.
  const app = readFileSync(join(PUBLIC, "app.js"), "utf8");
  expect(app).toContain('pageTiles("Plan")');
  expect(app).toContain('pageTiles("Operations")');
  expect(trends.route).toBe("more/trends");
});

/** A document for a window of `window` days over `days` days ending 2026-11-24, every series nothing. */
function doc(window: number, count: number) {
  const end = Date.parse("2026-11-24T00:00:00Z");
  const days = Array.from({ length: count }, (_, i) =>
    new Date(end - (count - 1 - i) * 86_400_000).toISOString().slice(0, 10),
  );
  const zeros = () => days.map(() => 0);
  const projects = [
    { id: "id", title: "id" },
    { id: "fleet", title: "Fleet" },
    { id: "atlas", title: "Atlas" },
    { id: "pm-stack", title: "PM stack" },
    { id: "mcpx", title: "mcpx" },
    { id: "other", title: "Everything else" },
  ];
  const by = () => Object.fromEntries(projects.map((p) => [p.id, zeros()])) as Record<string, number[]>;
  return {
    window,
    days,
    today: "2026-11-24",
    history: {
      from: "2026-08-26",
      whole: true,
      covered: "2026-08-26T00:00:00Z" as string | null,
      readTo: "2026-11-24T10:00:00Z",
      readAt: "2026-11-24T10:00:00Z" as string | null,
    },
    projects,
    merged: {
      all: zeros(),
      kinds: { feat: zeros(), fix: zeros(), docs: zeros(), test: zeros(), other: zeros() },
      projects: by(),
      minutes: days.map(() => null as number | null),
    },
    features: {
      projects: by(),
      latest: Object.fromEntries(
        projects.map((p) => [p.id, [] as { number: number; title: string; at: string; url: string }[]]),
      ),
    },
    issues: { opened: zeros(), closed: zeros() },
    bugs: { opened: zeros(), closed: zeros() },
    prs: { opened: zeros(), closed: zeros() },
    tasks: { all: zeros(), kinds: [] as { kind: string; days: number[] }[], since: null as string | null } as
      | { all: number[]; kinds: { kind: string; days: number[] }[]; since: string | null }
      | { error: string },
    partial: undefined as boolean | undefined,
    why: undefined as string | undefined,
  };
}

test("a week by the day: the window's own days are drawn, the week before is only what it is set against", () => {
  const d = doc(7, 14);
  // The week before: 10 merged a day (70). This week: 12 a day but today, which has 5 so far (77).
  d.merged.all = [10, 10, 10, 10, 10, 10, 10, 12, 12, 12, 12, 12, 12, 5];
  d.merged.kinds.feat = [4, 4, 4, 4, 4, 4, 4, 6, 6, 6, 6, 6, 6, 2];
  d.merged.kinds.fix = [6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 3];
  d.features.projects.id = [3, 3, 3, 3, 3, 3, 3, 2, 2, 2, 2, 2, 2, 1];
  d.features.projects["pm-stack"] = [1, 1, 1, 1, 1, 1, 1, 4, 4, 4, 4, 4, 4, 1];
  d.features.latest["pm-stack"] = [
    {
      number: 14379,
      title: "the app's look",
      at: "2026-11-24T13:36:00Z",
      url: "https://github.com/developing-today/code/pull/14379",
    },
  ];
  d.issues.closed = [0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1];
  d.issues.opened = [0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0];
  d.merged.minutes = [null, null, null, null, null, null, null, 30, 60, null, 90, 120, 600, 20];
  d.tasks = {
    all: [1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2],
    kinds: [
      { kind: "small fix", days: [0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1] },
      { kind: "feature slice", days: [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1] },
      { kind: "other", days: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
    ],
    since: "2026-11-11",
  };
  const sh = shape(d);
  expect([sh.n, sh.weekly, sh.days.length, sh.days[0], sh.days[6]]).toEqual([7, false, 7, "2026-11-18", "2026-11-24"]);
  expect(sh.labels).toEqual(["18 Nov", "19 Nov", "20 Nov", "21 Nov", "22 Nov", "23 Nov", "24 Nov"]);
  expect(sh.long[6]).toBe("Tue 24 Nov");
  // 77 against 70 is up 10%; features 8+6×... id 2×6+1 = 13 and pm-stack 4×6+1 = 25: 38 against 28 is up 36%;
  // issues closed 7 against none; tasks 14 against 7 is up 100%.
  expect(
    sh.kpis.map((k: { label: string; value: string; delta?: { text: string }; against: string }) => [
      k.label,
      k.value,
      k.delta?.text ?? null,
      k.against,
    ]),
  ).toEqual([
    ["Pull requests merged", "77", "▲ 10%", "on the 7 days before"],
    ["Features merged", "38", "▲ 36%", "on the 7 days before"],
    ["Issues closed", "7", "▲ from none", "on the 7 days before"],
    ["Tasks finished", "14", "▲ 100%", "on the 7 days before"],
    // The middle of 20, 30, 60, 90, 120 and 600 minutes is between 60 and 90: 75.
    ["Open to merged, typically", "75 min", null, "the middle pull request"],
  ]);
  expect(sh.kpis[0].spark).toEqual([12, 12, 12, 12, 12, 12, 5]);
  // Each kind and each project keeps its own colour, whatever is in the window.
  expect(sh.mergedByKind.map((s: { name: unknown; slot: unknown }) => [s.name, s.slot])).toEqual([
    ["features", 1],
    ["fixes", 2],
    ["docs", 3],
    ["tests", 4],
    ["everything else", "other"],
  ]);
  expect(sh.mergedByKind[0].values).toEqual([6, 6, 6, 6, 6, 6, 2]);
  expect(sh.featuresByProject.map((s: { name: unknown; slot: unknown }) => [s.name, s.slot])).toEqual([
    ["id", 1],
    ["Fleet", 2],
    ["Atlas", 3],
    ["PM stack", 4],
    ["mcpx", 5],
    ["Everything else", "other"],
  ]);
  // The projects that merged a feature, the most first, with what they merged.
  expect(
    sh.features.map((p: { title: unknown; count: number; latest: unknown[]; slot: unknown }) => [
      p.title,
      p.count,
      p.latest.length,
      p.slot,
    ]),
  ).toEqual([
    ["PM stack", 25, 1, 4],
    ["id", 13, 0, 1],
  ]);
  expect(
    sh.issues.map((s: { name: unknown; total: unknown; values: unknown[] }) => [s.name, s.total, s.values.length]),
  ).toEqual([
    ["opened", "2", 7],
    ["closed", "7", 7],
  ]);
  expect(sh.minutes[0].values).toEqual([30, 60, null, 90, 120, 600, 20]);
  expect(sh.minutes[0].total).toBe("75 min");
  // Tasks: kinds by name, "other" last and grey; the queue's first day is passed on.
  expect((sh.tasks ?? []).map((s: { name: unknown; slot: unknown }) => [s.name, s.slot])).toEqual([
    ["feature slice", 1],
    ["small fix", 2],
    ["other", "other"],
  ]);
  expect(sh.tasksSince).toBe("2026-11-11");
});

test("ninety days go by the week, the newest week whole; and with no days before them a total stands alone", () => {
  const d = doc(90, 90);
  d.merged.all = d.days.map(() => 1);
  const sh = shape(d);
  expect([sh.n, sh.weekly, sh.labels.length]).toEqual([90, true, 13]);
  // Thirteen weeks: twelve of seven days and the oldest of six.
  expect(sh.mergedByKind[4].values.length).toBe(13);
  expect(sh.long[12]).toBe("18 Nov to 24 Nov");
  expect(sh.long[0]).toBe("27 Aug to 1 Sep");
  expect(sh.labels[12]).toBe("24 Nov");
  expect([sh.kpis[0].value, sh.kpis[0].delta.text, sh.kpis[0].against]).toEqual(["90", "", "in 90 days"]);
  // The lines stay by the day.
  expect(sh.dayLabels.length).toBe(90);
  // A history shorter than the window is drawn as far as it goes.
  const short = shape(doc(30, 12));
  expect([short.n, short.weekly, short.labels.length]).toEqual([12, false, 12]);
});

test("the queue out of reach leaves the tasks out, and what cannot be seen yet is said first", () => {
  const d = doc(7, 14);
  d.tasks = { error: "queue store http 503" };
  const sh = shape(d);
  expect(sh.tasks).toBeNull();
  expect(sh.kpis.map((k: { label: string }) => k.label)).toEqual([
    "Pull requests merged",
    "Features merged",
    "Issues closed",
    "Open to merged, typically",
  ]);
  expect(notes(d)).toEqual([]);
  d.history.whole = false;
  d.history.covered = null;
  d.partial = true;
  d.why = "GitHub's history could not be read on: timed out";
  expect(notes(d)).toEqual([
    "The history is being read from GitHub for the first time, newest first. No day is whole yet. It reads on by itself; ↻ reads on now.",
    "GitHub's history could not be read on: timed out",
  ]);
  // Everything changed since noon on the 17th has been read: the 18th is the first whole day.
  d.history.covered = "2026-11-17T12:00:00Z";
  d.partial = undefined;
  expect(notes(d)).toEqual([
    "The history is being read from GitHub, newest first. Days from Wed 18 Nov on are whole; earlier ones are still coming in. It reads on by itself; ↻ reads on now.",
  ]);
  expect(wholeFrom("2026-11-17T12:00:00Z")).toBe("2026-11-18");
  // Read from the very start of a day, that day is whole; and across a month's end.
  expect(wholeFrom("2026-08-26T00:00:00Z")).toBe("2026-08-26");
  expect(wholeFrom("2026-10-31T23:59:59Z")).toBe("2026-11-01");
});
