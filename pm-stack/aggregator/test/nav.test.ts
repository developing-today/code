// #1135: navigation coherence. The nav bar groups its links by section
// instead of one scrolling strip, and every HTML route the server
// serves is reachable from the manifest — the bar, the dropdown, the
// /pages list, or a parent page's sub-route.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
  PAGE_GROUPS,
  PAGES,
  SUBROUTE_PARENTS,
  groupedPages,
  manifestRouteFor,
  pageByRoute,
} from "../src/pages.ts";
import { navLinks } from "../src/render.ts";

const SRC = join(import.meta.dir, "..", "src");
const serverSource = readFileSync(join(SRC, "server.ts"), "utf8");
const renderSource = readFileSync(join(SRC, "render.ts"), "utf8");

/** The CSS rule body for a selector in the stylesheet string. */
function cssRule(selector: string): string {
  const start = renderSource.indexOf(`\n${selector} {`);
  expect(start).toBeGreaterThan(-1);
  const end = renderSource.indexOf("}", start);
  return renderSource.slice(start, end);
}

/** Route literals the server compares a request path against. */
function serverPaths(): { exact: string[]; prefixes: string[] } {
  const exact = [...serverSource.matchAll(/path === "([^"]+)"/g)].map((m) => m[1]);
  const prefixes = [...serverSource.matchAll(/path\.startsWith\("([^"]+)"\)/g)].map((m) => m[1]);
  return { exact: [...new Set(exact)], prefixes: [...new Set(prefixes)] };
}

/** Routes that are not pages a person navigates to: JSON twins, POST
 * endpoints, datastar fragments, static assets, and the /board alias. */
const NON_PAGE = /^\/api\/|^\/frag\/|^\/assets\/|^\/sw\.js$|^\/board$/;

describe("the manifest groups every page (#1135)", () => {
  test("every page names one of PAGE_GROUPS, and groups have no page-less id", () => {
    const ids = new Set(PAGE_GROUPS.map((g) => g.id));
    for (const page of PAGES) expect(ids.has(page.group)).toBe(true);
    for (const group of groupedPages()) expect(group.pages.length).toBeGreaterThan(0);
  });

  test("groupedPages covers each page exactly once", () => {
    const flat = groupedPages().flatMap((g) => g.pages.map((p) => p.id));
    expect(flat.slice().sort()).toEqual(PAGES.map((p) => p.id).sort());
    expect(new Set(flat).size).toBe(PAGES.length);
  });
});

describe("the nav bar renders the groups (#1135)", () => {
  const html = navLinks("/quota");

  test("every page except /pages has a link, once, in the bar", () => {
    for (const page of PAGES.filter((p) => p.id !== "pages")) {
      const hits = html.split(`href="${page.route}"`).length - 1;
      expect(hits).toBe(1);
    }
    expect(html).not.toContain('href="/pages"');
  });

  test("each group with bar links renders its label, and the active page's section is marked", () => {
    // /pages is the manifest index: it lives in the dropdown and on
    // /pages itself, so its group needs no bar links (#1135).
    const inBar = groupedPages().filter((g) => g.pages.some((p) => p.id !== "pages"));
    for (const { group } of inBar) expect(html).toContain(`>${group.title}</span>`);
    expect(html).toContain('data-current="true"');
    expect(html.match(/data-current="true"/g)?.length).toBe(1);
    expect(html).toContain('href="/quota" class="active" aria-current="page"');
    expect(html.match(/aria-current="page"/g)?.length).toBe(1);
  });

  test("a sub-route page marks its parent's link and section active", () => {
    const sub = navLinks("/program/abc");
    expect(sub).toContain('href="/program" class="active" aria-current="page"');
    expect(sub.match(/aria-current="page"/g)?.length).toBe(1);
    expect(sub.match(/data-current="true"/g)?.length).toBe(1);
    expect(pageByRoute(manifestRouteFor("/program/abc") ?? "")?.group).toBe("work");
  });
});

describe("the bar wraps instead of scrolling sideways (#1135, 390px)", () => {
  test("the nav rule wraps and has no horizontal scroll", () => {
    const rule = cssRule("nav");
    expect(rule).toContain("flex-wrap: wrap");
    expect(rule).not.toContain("overflow-x: auto");
    expect(rule).not.toContain("overflow-x:auto");
    expect(cssRule(".navgroup")).toContain("flex-wrap: wrap");
  });

  test("the phone breakpoint tightens the bar instead of widening it", () => {
    const mobile = renderSource.slice(renderSource.indexOf("@media (max-width: 720px)"));
    expect(mobile).toContain("nav a { padding:");
    expect(mobile).not.toContain("overflow-x");
  });
});

describe("every server route is reachable from the manifest (#1135)", () => {
  const { exact, prefixes } = serverPaths();

  test("the server serves every manifest route and its JSON twin", () => {
    for (const page of PAGES) {
      expect(exact.some((p) => p === page.route) || prefixes.some((p) => page.route.startsWith(p))).toBe(true);
      if (page.api) expect(exact).toContain(page.api);
    }
  });

  test("no HTML route is orphaned", () => {
    const orphans = exact.filter((p) => !NON_PAGE.test(p) && manifestRouteFor(p) === null);
    expect(orphans).toEqual([]);
  });

  test("no HTML sub-route prefix is orphaned", () => {
    const subPrefixes = prefixes.filter((p) => !NON_PAGE.test(p));
    const known = SUBROUTE_PARENTS.map((s) => s.prefix);
    for (const prefix of subPrefixes) expect(known).toContain(prefix);
  });

  test("reachability follows the nav: parent pages exist and are linked", () => {
    const bar = navLinks("/");
    for (const { parent } of SUBROUTE_PARENTS) {
      expect(pageByRoute(parent)).toBeDefined();
      expect(bar).toContain(`href="${parent}"`);
    }
  });
});
