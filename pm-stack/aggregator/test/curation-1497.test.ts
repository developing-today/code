// #1497: the page-list curation, confirmed 2026-11-01 (field review
// 2026-10-27 + triage task 2593). The decision table is data here so a
// future manifest edit that drifts from it fails loudly.
import { describe, expect, test } from "bun:test";
import type { Comment, ProjectPoolItem, Provenance } from "../src/model.ts";
import { buildBriefs } from "../src/brief.ts";
import { PAGES, SUBROUTE_PARENTS, manifestRouteFor, pageByRoute, primaryGroupedPages } from "../src/pages.ts";
import { buildProgramData } from "../src/program.ts";
import { navLinks, pageDropdown, renderPages, renderProgram } from "../src/render.ts";
import { STANDING_IDENTITIES } from "../src/registry.ts";

const PROV: Provenance = {
  board: "x",
  roadmap: "x",
  items: "x",
  comments: "x",
  quota: "x",
  usage: "x",
  attention: "x",
  orchestration: "x",
  dns: "x",
  registry: "x",
  aa: "x",
};

const DECISIONS: Record<string, "primary" | "dropdown"> = {
  overview: "primary",
  today: "primary",
  attention: "primary",
  board: "dropdown",
  roadmap: "primary",
  search: "dropdown",
  quota: "primary",
  dns: "dropdown",
  "aa-explorer": "dropdown",
  sessions: "dropdown",
  program: "primary",
  queue: "primary",
  health: "primary",
  pages: "dropdown",
  // Post-#1497 addition (#3676, 2026-11-01): Alerts entered at
  // dropdown tier; Drew's ruling the same day (he could not find the
  // ack surface from his phone) promotes it to primary — a curation
  // decision, recorded here like the others.
  alerts: "primary",
  // Post-#1497 addition (#1210, 2026-11-01): the comments queue
  // enters at dropdown tier — no curation ruling promotes it, and
  // new pages do not take primary slots without one.
  "comments-queue": "dropdown",
  // Post-#1497 addition (#1739, 2026-11-02): the events page
  // enters at dropdown tier — no curation ruling promotes it, and
  // new pages do not take primary slots without one.
  events: "dropdown",
  // Post-#1497 addition (#1282, 2026-11-03): the source-conflict inbox
  // enters at dropdown tier — a triage queue, not a primary surface.
  conflicts: "dropdown",
  // Post-#1497 addition (#3319, 2026-11-03): the notifications
  // measurement page enters at dropdown tier — no curation ruling
  // promotes it, and new pages do not take primary slots without one.
  notifications: "dropdown",
  // Post-#1497 addition (#1278, 2026-11-03): the scheduled-digest
  // archive page enters at dropdown tier — no curation ruling promotes
  // it, and new pages do not take primary slots without one.
  digest: "dropdown",
  // Post-#1497 addition (#8867, 2026-11-12): the decision-sets index
  // entered at dropdown tier; #9354 (Drew 2026-11-13, he could not
  // find the question stack) promotes it to primary — a curation
  // ruling recorded here like the others.
  decisions: "primary",
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
    expect(flat.sort()).toEqual(
      PAGES.filter((p) => p.nav === "primary")
        .map((p) => p.id)
        .sort(),
    );
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

  test("renderProgram forwards prebuilt comment-backed briefs to the pool cards (#1279)", () => {
    const pool: ProjectPoolItem[] = [
      {
        id: "PVTI_y",
        url: "https://github.com/orgs/developing-today/projects/4?pane=issue&itemId=2",
        projectUrl: "https://github.com/orgs/developing-today/projects/4",
        ref: "developing-today/code#2",
        repo: "developing-today/code",
        number: 2,
        issueUrl: "https://github.com/developing-today/code/issues/2",
        title: "briefed pool card",
        state: "open",
        contentKind: "issue",
        // #5082: briefs render on full cards (active work + epics);
        // a Todo task renders compact, so the forwarded-brief
        // fixture is active work.
        status: "In Progress",
        area: "pm-dashboard",
        horizon: null,
        kind: null,
      },
    ];
    const briefs = buildBriefs(pool, {
      comments: [
        {
          id: "c1",
          ref: "developing-today/code#2",
          kind: "issue_comment",
          author: "alice",
          body: "working on it",
          createdAt: "2026-10-30T00:00:00Z",
          url: "https://github.com/developing-today/code/issues/2#issuecomment-1",
          deleted: false,
        } as Comment,
      ],
    });
    const items = new Map();
    const data = { ...buildProgramData(items, pool, null), projects: [] };
    const withBriefs = renderProgram(data, items, PROV, undefined, { pool, area: null, briefs });
    // The comment-backed brief names the last actor; the comment-less
    // fallback built inside projectsBodyHtml cannot.
    expect(withBriefs).toContain("Last activity by alice");
    const without = renderProgram(data, items, PROV, undefined, { pool, area: null });
    expect(without).not.toContain("Last activity by alice");
  });

  test("pool rows: active work and epics render full cards, the rest render compact (#5082)", () => {
    const mk = (n: number, title: string, status: string, kind: string | null): ProjectPoolItem => ({
      id: `PVTI_${n}`,
      url: `https://github.com/orgs/developing-today/projects/4?pane=issue&itemId=${n}`,
      projectUrl: "https://github.com/orgs/developing-today/projects/4",
      ref: `developing-today/code#${n}`,
      repo: "developing-today/code",
      number: n,
      issueUrl: `https://github.com/developing-today/code/issues/${n}`,
      title,
      state: "open",
      contentKind: "issue",
      status,
      area: "pm-dashboard",
      horizon: null,
      kind,
    });
    const pool: ProjectPoolItem[] = [
      mk(10, "compact todo marker", "Todo", null),
      mk(11, "full active marker", "In Progress", null),
      mk(12, "full epic marker", "Todo", "Epic"),
      mk(13, "compact done marker", "Done", null),
    ];
    const items = new Map();
    const data = { ...buildProgramData(items, pool, null), projects: [] };
    const html = renderProgram(data, items, PROV, undefined, { pool, area: null, identities: STANDING_IDENTITIES });
    // Every item is still listed.
    for (const t of ["compact todo marker", "full active marker", "full epic marker", "compact done marker"]) {
      expect(html).toContain(t);
    }
    // Full cards (active + epic) carry the brief panel and the
    // status-request form; compact rows carry neither.
    expect(html.split('class="brief"').length - 1).toBe(2);
    expect(html.split('class="aform"').length - 1).toBe(2);
    const todoAt = html.indexOf("compact todo marker");
    const todoCard = html.slice(
      html.lastIndexOf('<div class="card">', todoAt),
      html.indexOf('<div class="card">', todoAt),
    );
    expect(todoCard).not.toContain('class="brief"');
    expect(todoCard).not.toContain('class="aform"');
    expect(todoCard).toContain("developing-today/code#10");
  });
});
