import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// #1180 mobile audit (390x844): control-class tap targets carry a 40px
// floor in the stylesheet. Static assertions, same style as nav.test.ts —
// the rendered measurements live on issue #1180.
const SRC = join(import.meta.dir, "..", "src");
const renderSource =
  readFileSync(join(SRC, "ui.css"), "utf8") + "\n" + readFileSync(join(SRC, "render.ts"), "utf8");

function cssRule(selector: string): string {
  const start = renderSource.indexOf(`\n${selector} {`);
  expect(start).toBeGreaterThan(-1);
  const end = renderSource.indexOf("}", start);
  return renderSource.slice(start, end);
}

describe("tap targets are at least 40px tall (#1180)", () => {
  test("quick-action chips", () => {
    expect(cssRule(".qchip")).toContain("min-height: 40px");
  });
  test("nav links", () => {
    expect(cssRule("nav a")).toContain("min-height: 40px");
  });
  test("quota-block checkbox labels", () => {
    expect(cssRule(".qblock label")).toContain("min-height: 40px");
  });
  test("chip links and explorer filter chips", () => {
    expect(cssRule(".chips a")).toContain("min-height: 40px");
    expect(cssRule(".xchip")).toContain("min-height: 40px");
  });
  test("native controls, disclosures, and checkbox labels have a mobile floor", () => {
    expect(renderSource).toContain(
      'button, select, input:not([type="hidden"]):not([type="checkbox"]):not([type="radio"]), textarea, summary { min-height: 40px; }',
    );
    expect(renderSource).toContain(
      'label:has(input[type="checkbox"]), label:has(input[type="radio"]) { display: inline-flex; align-items: center; min-height: 40px; }',
    );
    expect(renderSource).toContain("body { overflow-x: clip; }");
  });
  test("mobile anchor offset clears the wrapped sticky nav", () => {
    expect(renderSource).toContain("[id] { scroll-margin-top: 192px; }");
  });
  test("wide project and alert tables live in intentional scroll wrappers", () => {
    expect(renderSource).toContain('<div class="tblwrap"><table><caption>Coverage — pool vs backlog by repository');
    expect(renderSource).toContain('<div class="tblwrap"><table><caption>Recent alert-ledger transitions');
  });
  test("attention headers wrap and item session rows stay within the mobile card", () => {
    expect(renderSource).toMatch(/\.attention-card-header \{[^}]*flex-wrap: wrap;/);
    expect(renderSource).toMatch(/\.attention-card-header > strong \{[^}]*min-width: 0;/);
    expect(renderSource).toMatch(/\.item-session-records > \.session-record-line \{[^}]*max-width: 100%;/);
    expect(renderSource).toMatch(/\.item-session-records \.session-record-meta \{[^}]*overflow-wrap: anywhere;/);
    expect(renderSource).toMatch(/\.item-session-records \.session-record-line \.badge \{[^}]*white-space: normal;/);
    expect(renderSource).toMatch(/\.item-session-records \.session-record-line \.badge \{[^}]*overflow-wrap: anywhere;/);
    expect(renderSource).toContain('<div class="attention-card-header">');
    expect(renderSource).toContain('<div class="item-session-records">');
  });
  test("interactive AA chart points have a large touch target", () => {
    expect(renderSource).toContain('class="xpt-hit" cx="');
    expect(renderSource).toContain('r="40"/>');
  });
  test("pre blocks scroll inside themselves, not the page", () => {
    expect(cssRule("pre")).toContain("overflow-x: auto");
  });
});
