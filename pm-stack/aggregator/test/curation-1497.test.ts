// #1497: the page-list curation, confirmed 2026-11-01 (field review
// 2026-10-27 + triage task 2593). The decision table is data here so a
// future manifest edit that drifts from it fails loudly.
import { describe, expect, test } from "bun:test";
import type { ProjectPoolItem, Provenance } from "../src/model.ts";
import { PAGES, SUBROUTE_PARENTS, manifestRouteFor, pageByRoute, primaryGroupedPages } from "../src/pages.ts";
import { buildProgramData } from "../src/program.ts";
import { navLinks, pageDropdown, renderPages, renderProgram } from "../src/render.ts";

const PROV: Provenance = {
  board: "x", roadmap: "x", items: "x", comments: "x", quota: "x",
  usage: "x", attention: "x", orchestration: "x", dns: "x",
  registry: "x", aa: "x",
};

const DECISIONS: Record<string, "primary" | "dropdown"> = {
  overview: "primary",
  today: "primary",
  attention: "primary",
  board: "dropdown",
  roadmap: "primary",
  quota: "primary",
  dns: "dropdown",
  "aa-explorer": "dropdown",
  sessions: "dropdown",
  program: "primary",
  health: "primary",
  pages: "dropdown",
};

describe("the #1497 decision table is applied to the manifest", () => {
  test("every manifest page carries its recorded tier, and the table covers the manifest", () => {
    expect(PAGES.map((p) => p.id).sort()).toEqual(Object.keys(DECISIONS).sort());
    for (const page of PAGES) expect(page.nav).toBe(DECISIONS[page.id]);
  });

  test("Projects is merged: no manifest entry, /projects resolves to Program", () => {
    expect(PAGES.find((p) => p.id === "projects")).toBeUndefined();
    expect(pageByRoute("/projects")).toBeUndefined();
    expect(manifestRouteFor("/projects")).toBe("/program");
    const entry = SUBROUTE_PARENTS.find((s) => s.name === "projects-merge");
    expect(entry?.parent).toBe("/program");
  });

  test("the manifest names every subroute entry (item, discuss, program-project, service-detail)", () => {
    expect(SUBROUTE_PARENTS.map((s) => s.name)).toEqual(
      expect.arrayContaining(["item", "discuss", "program-project", "service-detail", "projects-merge"]),
    );
  });

  test("the bar shows exactly the primary tier; the dropdown shows everything", () => {
    const bar = navLinks("/");
    const drop = pageDropdown("/");
    for (const page of PAGES) {
      if (page.nav === "primary") expect(bar).toContain(`href="${page.route}"`);
      else expect(bar).not.toContain(`href="${page.route}"`);
      expect(drop).toContain(`value="${page.route}"`);
    }
    const flat = primaryGroupedPages().flatMap((g) => g.pages.map((p) => p.id));
    expect(flat.sort()).toEqual(PAGES.filter((p) => p.nav === "primary").map((p) => p.id).sort());
  });
});

describe("the /pages live index (#1497)", () => {
  test("renders last-rendered + ok from the registry, and says so for unrendered pages", () => {
    const html = renderPages(PROV, [
      { route: "/today", title: "Today", lastRenderedAt: "2026-11-01T07:00:00Z", ok: true, ageMs: 60_000 },
      { route: "/dns", title: "DNS register", lastRenderedAt: "2026-11-01T07:00:00Z", ok: false, ageMs: 60_000 },
    ]);
    expect(html).toContain("Dropdown-only");
    expect(html).toContain("Primary nav");
    expect(html).toContain("last render failed");
    expect(html).toContain("not rendered since this process started");
  });
});

describe("the Projects merge into Program (#1497)", () => {
  test("renderProgram with a pool section renders the pool cards under #projects-pool", () => {
    const pool: ProjectPoolItem[] = [
      {
        id: "PVTI_x",
        url: "https://github.com/orgs/developing-today/projects/4?pane=issue&itemId=1",
        projectUrl: "https://github.com/orgs/developing-today/projects/4",
        ref: "developing-today/code#1",
        repo: "developing-today/code",
        number: 1,
        issueUrl: "https://github.com/developing-today/code/issues/1",
        title: "pool card marker",
        state: "open",
        contentKind: "issue",
        status: "Todo",
        area: "pm-dashboard",
        horizon: null,
        kind: null,
      },
    ];
    const items = new Map();
    const data = { ...buildProgramData(items, pool, null), projects: [] };
    const html = renderProgram(data, items, PROV, undefined, { pool, area: null });
    expect(html).toContain('id="projects-pool"');
    expect(html).toContain("pool card marker");
    // Area chips in the merged section point back at /program, not the redirect.
    expect(html).toContain('href="/program?area=pm-dashboard"');
    expect(html).not.toContain('href="/projects?area=');
  });
});
