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
}

export const PAGES: PageDef[] = [
  {
    id: "overview",
    title: "Overview",
    route: "/",
    api: "/api/overview.json",
    blurb: "The stack front door (#1134): needs-you, projects, quotas, and health at a glance.",
  },
  {
    id: "today",
    title: "Today",
    route: "/today",
    api: "/api/today.json",
    blurb: "The day's top objectives + per-project momentum.",
    group: "overview",
  },
  {
    id: "attention",
    title: "Needs you",
    route: "/attention",
    api: "/api/attention.json",
    blurb: "NEEDS USER vs NEEDS ORCHESTRATOR, and every live session.",
    group: "overview",
  },
  {
    id: "board",
    title: "Board",
    route: "/",
    api: "/api/board.json",
    blurb: "The curated cross-repo WIP board.",
    group: "overview",
  },
  {
    id: "roadmap",
    title: "Roadmap",
    route: "/roadmap",
    api: "/api/roadmap.json",
    blurb: "The program roadmap at every grain.",
    group: "overview",
  },
  {
    id: "quota",
    title: "Usage & billing",
    route: "/quota",
    api: "/api/quota.json",
    blurb: "Token quotas, PAYG dollars, and every SaaS service's usage vs its limits.",
    group: "ops",
  },
  {
    id: "dns",
    title: "DNS register",
    route: "/dns",
    api: "/api/dns.json",
    blurb: "Every domain: next renewal, unused, earmarked. (#902's register)",
    group: "ops",
  },
  {
    id: "aa-explorer",
    title: "Cost Explorer",
    route: "/aa",
    api: "/api/aa.json",
    blurb:
      "Two tabs over the fleet's cost data: AA Explorer (Artificial Analysis measurements, frontier charts, plan/API/per-task efficiency) and Model Explorer (every model in the fleet's catalogs, its pool seats, plan, and measured value).",
    group: "ops",
  },
  {
    id: "sessions",
    title: "Sessions",
    route: "/sessions",
    api: "/api/registry.json",
    blurb: "The agent/session identity registry: who ran what, tokens, task links.",
    group: "work",
  },
  {
    id: "projects",
    title: "Projects",
    route: "/projects",
    api: "/api/projects.json",
    blurb: "The live GitHub Project pool (#4): every item, its Project fields, and Project-vs-derived disagreements.",
    group: "work",
  },
  {
    id: "program",
    title: "Program status",
    route: "/program",
    api: "/api/program.json",
    blurb:
      "The projects hub: every project's status, roadmap, and tracked work, one page per project under /program/<id> — fed from the project registry, the tracked issues, and the Project pool.",
    group: "work",
  },
  {
    id: "health",
    title: "Health",
    route: "/health",
    api: "/api/health",
    blurb:
      "Service & page health (#1147): live probes for every stack service, feed section ages, last job runs, this dashboard's render history — and the services & ports topology (#1137).",
    group: "ops",
  },
  {
    id: "pages",
    title: "All pages",
    route: "/pages",
    blurb: "This manifest as a page — every registered view in one place.",
    group: "system",
  },
];

export function pageByRoute(route: string): PageDef | undefined {
  return PAGES.find((p) => p.route === route);
}

/** The nav's sections in bar order, each with the pages that belong to it (#1135). */
export function groupedPages(): { group: PageGroupDef; pages: PageDef[] }[] {
  return PAGE_GROUPS.map((group) => ({ group, pages: PAGES.filter((p) => p.group === group.id) }));
}

/**
 * Sub-routes a manifest page owns (#1135 reachability): /program/<id> is
 * reached from /program's nav link, /item/<subject> from the board, and
 * so on. Prefix → the manifest route that reaches it.
 */
export const SUBROUTE_PARENTS: { prefix: string; parent: string }[] = [
  { prefix: "/item/", parent: "/" },
  { prefix: "/discuss/", parent: "/" },
  { prefix: "/program/", parent: "/program" },
  { prefix: "/quota/service/", parent: "/quota" },
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
