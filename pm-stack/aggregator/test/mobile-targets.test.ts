import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// #1180 mobile audit (390x844): control-class tap targets carry a 40px
// floor in the stylesheet. Static assertions, same style as nav.test.ts —
// the rendered measurements live on issue #1180.
const SRC = join(import.meta.dir, "..", "src");
const renderSource = readFileSync(join(SRC, "render.ts"), "utf8");

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
  test("pre blocks scroll inside themselves, not the page", () => {
    expect(cssRule("pre")).toContain("overflow-x: auto");
  });
});
