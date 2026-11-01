// The page manifest (#906 §1): every dashboard page is a registered
// entry here — the nav bar and the home-page dropdown are rendered
// from this list, never hard-coded per view.
//
// Adding a page (the ~30-minute recipe, also in the README):
//   1. Add one entry below: id, title, route, api (its /api/*.json
//      twin), blurb, group (one of PAGE_GROUPS — the nav section).
//   2. Add a render function in src/render.ts and one route branch in
//      src/server.ts (plus the /api twin route).
//   3. If the page needs data, add it to the Source interface
//      (src/source.ts) with a live reader + fixture fallback.
// Nothing else registers the page — nav, dropdown, and this manifest
// are the same list.

/**
 * The nav's sections (#1135). The bar renders these in order, each with
 * its own label, so ten links never have to fit one scrolling strip: the
 * groups wrap instead. A page's `group` must name one of these ids.
 */
export interface PageGroupDef {
  id: string;
  title: string;
}

export const PAGE_GROUPS: PageGroupDef[] = [
  { id: "overview", title: "Overview" },
  { id: "work", title: "Work" },
  { id: "ops", title: "Ops" },
  { id: "system", title: "System" },
];

export interface PageDef {
  id: string;
  title: string;
  route: string;
  /** Machine-first twin of the page, when one exists. */
  api?: string;
  blurb: string;
  /** Nav section this page renders under (#1135). */
  group: string;
  /**
   * Nav tier (#1497 curation, confirmed 2026-11-01): "primary" pages
   * earn a slot in the top nav bar; "dropdown" pages are reachable
   * from the Pages dropdown (and their drill-down parents) only.
   */
  nav: "primary" | "dropdown";
}

// The #1497 curation (field review 2026-10-27, confirmed by the
// triage reviewer 2026-11-01, task 2593): Today/Needs you/Roadmap/
// Usage & billing/Program/Health/Overview keep primary-nav slots;
// Board, DNS, Cost Explorer, and Sessions are dropdown-only
// reference/lookup pages; All pages is dropdown-only and renders the
// live render registry; Projects is MERGED into Program status —
// its pool view is a section of /program, /projects redirects there,
// and /api/projects.json stays as the machine twin.
export const PAGES: PageDef[] = [
  {
    id: "overview",
    title: "Overview",
    route: "/",
    api: "/api/overview.json",
    blurb: "The stack front door (#1134): needs-you, projects, quotas, and health at a glance.",
    group: "overview",
    nav: "primary",
  },
  {
    id: "today",
    title: "Today",
    route: "/today",
    api: "/api/today.json",
    blurb: "The day's top objectives + per-project momentum.",
    group: "overview",
    nav: "primary",
  },
  {
    id: "attention",
    title: "Needs you",
    route: "/attention",
    api: "/api/attention.json",
    blurb: "NEEDS USER vs NEEDS ORCHESTRATOR, and every live session.",
    group: "overview",
    nav: "primary",
  },
  {
    id: "board",
    title: "Board",
    route: "/board",
    api: "/api/board.json",
    blurb: "The curated cross-repo WIP board.",
    group: "overview",
    nav: "dropdown",
  },
  {
    id: "roadmap",
    title: "Roadmap",
    route: "/roadmap",
    api: "/api/roadmap.json",
    blurb: "The program roadmap at every grain.",
    group: "overview",
    nav: "primary",
  },
  {
    id: "quota",
    title: "Usage & billing",
    route: "/quota",
    api: "/api/quota.json",
    blurb: "Token quotas, PAYG dollars, and every SaaS service's usage vs its limits.",
    group: "ops",
    nav: "primary",
  },
  {
    id: "dns",
    title: "DNS register",
    route: "/dns",
    api: "/api/dns.json",
    blurb: "Every domain: next renewal, unused, earmarked. (#902's register)",
    group: "ops",
    nav: "dropdown",
  },
  {
    id: "aa-explorer",
    title: "Cost Explorer",
    route: "/aa",
    api: "/api/aa.json",
    blurb:
      "Two tabs over the fleet's cost data: AA Explorer (Artificial Analysis measurements, frontier charts, plan/API/per-task efficiency) and Model Explorer (every model in the fleet's catalogs, its pool seats, plan, and measured value).",
    group: "ops",
    nav: "dropdown",
  },
  {
    id: "sessions",
    title: "Sessions",
    route: "/sessions",
    api: "/api/registry.json",
    blurb: "The agent/session identity registry: who ran what, tokens, task links.",
    group: "work",
    nav: "dropdown",
  },
  {
    id: "program",
    title: "Program status",
    route: "/program",
    api: "/api/program.json",
    blurb:
      "The projects hub: every project's status, roadmap, and tracked work, one page per project under /program/<id> — fed from the project registry, the tracked issues, and the Project pool.",
    group: "work",
    nav: "primary",
  },
  {
    id: "health",
    title: "Health",
    route: "/health",
    api: "/api/health",
    blurb:
      "Service & page health (#1147): live probes for every stack service, feed section ages, last job runs, this dashboard's render history — and the services & ports topology (#1137).",
    group: "ops",
    nav: "primary",
  },
  {
    id: "pages",
    title: "All pages",
    route: "/pages",
    blurb: "This manifest as a page — every registered view in one place.",
    group: "system",
    nav: "dropdown",
  },
];

export function pageByRoute(route: string): PageDef | undefined {
  return PAGES.find((p) => p.route === route);
}

/** The nav's sections in bar order, each with the pages that belong to it (#1135). */
export function groupedPages(): { group: PageGroupDef; pages: PageDef[] }[] {
  return PAGE_GROUPS.map((group) => ({ group, pages: PAGES.filter((p) => p.group === group.id) }));
}

/** The top nav bar's sections (#1497): the grouped model filtered to
 * primary-tier pages. The Pages dropdown keeps the full groupedPages()
 * list, so a demoted page loses its bar slot, never its reachability. */
export function primaryGroupedPages(): { group: PageGroupDef; pages: PageDef[] }[] {
  return PAGE_GROUPS.map((group) => ({
    group,
    pages: PAGES.filter((p) => p.group === group.id && p.nav === "primary"),
  })).filter((g) => g.pages.length > 0);
}

/**
 * Sub-routes a manifest page owns (#1135 reachability): /program/<id> is
 * reached from /program's nav link, /item/<subject> from the board, and
 * so on. Prefix → the manifest route that reaches it.
 */
export const SUBROUTE_PARENTS: { name: string; prefix: string; parent: string }[] = [
  { name: "item", prefix: "/item/", parent: "/" },
  { name: "discuss", prefix: "/discuss/", parent: "/" },
  { name: "program-project", prefix: "/program/", parent: "/program" },
  { name: "service-detail", prefix: "/quota/service/", parent: "/quota" },
  // #1497: Projects merged into Program — /projects redirects to the
  // pool section of /program, which is its manifest parent.
  { name: "projects-merge", prefix: "/projects", parent: "/program" },
];

/**
 * The manifest route that reaches `path` — the page itself, or the page
 * whose link leads to a sub-route (#1147 render registry, #1135
 * reachability). Null for routes outside the manifest (assets, /frag/*,
 * the POST endpoints).
 */
export function manifestRouteFor(path: string): string | null {
  const exact = PAGES.find((p) => p.route === path);
  if (exact) return exact.route;
  return SUBROUTE_PARENTS.find((s) => path.startsWith(s.prefix))?.parent ?? null;
}
