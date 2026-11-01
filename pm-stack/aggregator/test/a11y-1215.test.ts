// #1215: aggregator accessibility pass — a real light theme under
// prefers-color-scheme, non-colour cues for colour-coded states, one
// navigation model for the bar and the Pages dropdown, and captions /
// scoped headers on every aggregator table.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { PAGES } from "../src/pages.ts";
import { navLinks, pageDropdown } from "../src/render.ts";

const renderSource = readFileSync(join(import.meta.dir, "..", "src", "render.ts"), "utf8");

describe("light theme (#1215)", () => {
  const light = renderSource.slice(
    renderSource.indexOf("@media (prefers-color-scheme: light)"),
    renderSource.indexOf("@media (max-width: 720px)"),
  );

  test("a light palette block exists under prefers-color-scheme", () => {
    expect(renderSource).toContain("@media (prefers-color-scheme: light)");
    expect(light.length).toBeGreaterThan(500);
  });

  test("the light block covers the core surfaces: body, cards, badges, banners, tables, links", () => {
    for (const sel of ["body {", ".card {", ".badge", ".banner", "th {", "a {", "nav {"]) {
      expect(light).toContain(sel);
    }
    // Light surfaces are actually light, dark text on them.
    expect(light).toContain("background: #ffffff");
    expect(light).toContain("color: #1f2328");
  });

  test("the dark palette is unchanged as the default", () => {
    expect(renderSource).toContain("body { margin: 0; font: 14px/1.45 system-ui, -apple-system, sans-serif; background: #0e1117; color: #e6edf3; }");
  });
});

describe("non-colour cues (#1215)", () => {
  test("colour-coded state badges also differ by icon prefix", () => {
    for (const sel of [".badge.walled::before", ".badge.blocked::before", ".badge.drew::before", ".badge.act-pending::before", ".badge.act-applied::before", ".badge.healthy::before", ".badge.stale::before"]) {
      expect(renderSource).toContain(sel);
    }
    expect(renderSource).toContain(String.raw`content: "\\26a0`);
  });

  test("status dots also differ by shape, not only colour", () => {
    expect(renderSource).toContain(".dot.down, .dot.invalid { border-radius: 2px; }");
    expect(renderSource).toContain(".dot.stale, .dot.absent { border-radius: 2px; transform: rotate(45deg); }");
    expect(renderSource).toContain(".dot.fixtures { background: transparent; border: 2px solid #bc8cff; }");
  });

  test("acknowledged cards carry a text badge, not just reduced opacity", () => {
    expect(renderSource).toContain('<span class="badge acked">Acknowledged</span>');
    expect(renderSource).toContain(".badge.acked {");
  });
});

describe("one navigation model (#1215)", () => {
  const hrefs = [...navLinks("/quota").matchAll(/href="([^"]+)"/g)].map((m) => m[1]);
  const values = [...pageDropdown("/quota").matchAll(/<option value="([^"]+)"/g)].map((m) => m[1]).filter((v) => v !== "");

  test("bar and dropdown list the identical destination set", () => {
    expect([...new Set(hrefs)].sort()).toEqual([...new Set(values)].sort());
    expect(new Set(hrefs).size).toBe(PAGES.length);
  });

  test("both include /pages and every manifest route", () => {
    for (const page of PAGES) {
      expect(hrefs).toContain(page.route);
      expect(values).toContain(page.route);
    }
    expect(hrefs).toContain("/pages");
    expect(values).toContain("/pages");
  });

  test("the dropdown groups its options under the same sections as the bar", () => {
    const dropdown = pageDropdown("/");
    for (const label of ["Overview", "Work", "Ops", "System"]) {
      expect(dropdown).toContain(`<optgroup label="${label}">`);
      expect(navLinks("/")).toContain(`>${label}</span>`);
    }
  });
});

describe("table semantics (#1215)", () => {
  test("every <table> in render.ts opens with a <caption>", () => {
    const tables = [...renderSource.matchAll(/<table[^>]*>/g)];
    expect(tables.length).toBeGreaterThan(20);
    for (const t of tables) {
      const after = renderSource.slice(t.index + t[0].length, t.index + t[0].length + 120);
      expect(after.startsWith("<caption>") || after.includes("<caption>' + esc(title) + '</caption>")).toBe(true);
    }
    expect(renderSource.match(/<caption/g)?.length).toBe(tables.length);
  });

  test("every header cell carries a scope", () => {
    const bare = [...renderSource.matchAll(/<th(?=[ >])/g)].filter((m) => {
      const tag = renderSource.slice(m.index, renderSource.indexOf(">", m.index));
      return !tag.includes("scope=");
    });
    expect(bare).toEqual([]);
    expect(renderSource).toContain('<th scope="col">');
    expect(renderSource).toContain('<th scope="row">');
  });
});
