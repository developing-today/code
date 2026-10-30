// The page manifest (#906 §1): every dashboard page is a registered
// entry here — the nav bar and the home-page dropdown are rendered
// from this list, never hard-coded per view.
//
// Adding a page (the ~30-minute recipe, also in the README):
//   1. Add one entry below: id, title, route, api (its /api/*.json
//      twin), blurb.
//   2. Add a render function in src/render.ts and one route branch in
//      src/server.ts (plus the /api twin route).
//   3. If the page needs data, add it to the Source interface
//      (src/source.ts) with a live reader + fixture fallback.
// Nothing else registers the page — nav, dropdown, and this manifest
// are the same list.

export interface PageDef {
  id: string;
  title: string;
  route: string;
  /** Machine-first twin of the page, when one exists. */
  api?: string;
  blurb: string;
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
  },
  {
    id: "attention",
    title: "Needs you",
    route: "/attention",
    api: "/api/attention.json",
    blurb: "NEEDS USER vs NEEDS ORCHESTRATOR, and every live session.",
  },
  { id: "board", title: "Board", route: "/board", api: "/api/board.json", blurb: "The curated cross-repo WIP board." },
  {
    id: "roadmap",
    title: "Roadmap",
    route: "/roadmap",
    api: "/api/roadmap.json",
    blurb: "The program roadmap at every grain.",
  },
  {
    id: "quota",
    title: "Usage & billing",
    route: "/quota",
    api: "/api/quota.json",
    blurb: "Token quotas, PAYG dollars, and every SaaS service's usage vs its limits.",
  },
  {
    id: "dns",
    title: "DNS register",
    route: "/dns",
    api: "/api/dns.json",
    blurb: "Every domain: next renewal, unused, earmarked. (#902's register)",
  },
  {
    id: "aa-explorer",
    title: "Cost Explorer",
    route: "/aa",
    api: "/api/aa.json",
    blurb:
      "Two tabs over the fleet's cost data: AA Explorer (Artificial Analysis measurements, frontier charts, plan/API/per-task efficiency) and Model Explorer (every model in the fleet's catalogs, its pool seats, plan, and measured value).",
  },
  {
    id: "sessions",
    title: "Sessions",
    route: "/sessions",
    api: "/api/registry.json",
    blurb: "The agent/session identity registry: who ran what, tokens, task links.",
  },
  {
    id: "projects",
    title: "Projects",
    route: "/projects",
    api: "/api/projects.json",
    blurb: "The live GitHub Project pool (#4): every item, its Project fields, and Project-vs-derived disagreements.",
  },
  {
    id: "program",
    title: "Program status",
    route: "/program",
    api: "/api/program.json",
    blurb:
      "The projects hub: every project's status, roadmap, and tracked work, one page per project under /program/<id> — fed from the project registry, the tracked issues, and the Project pool.",
  },
  {
    id: "health",
    title: "Health",
    route: "/health",
    api: "/api/health",
    blurb:
      "Service & page health (#1147): live probes for every stack service, feed section ages, last job runs, this dashboard's render history — and the services & ports topology (#1137).",
  },
  {
    id: "pages",
    title: "All pages",
    route: "/pages",
    blurb: "This manifest as a page — every registered view in one place.",
  },
];

export function pageByRoute(route: string): PageDef | undefined {
  return PAGES.find((p) => p.route === route);
}
