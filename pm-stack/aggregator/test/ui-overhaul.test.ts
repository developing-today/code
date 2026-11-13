// Phase C (#6153): the Tailwind + DaisyUI overhaul — the page shell links
// the committed stylesheet build instead of inlining CSS, the theme
// toggle is Datastar-driven, the stylesheet source carries the DaisyUI
// plugin, the server serves the build and gives slow fragments a 60s
// idle timeout, and the fragment contract (ids + patch re-GET hooks)
// is untouched.
import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { renderSearch } from "../src/render.ts";

const SRC = join(import.meta.dir, "..", "src");
const renderSource = readFileSync(join(SRC, "render.ts"), "utf8");
const serverSource = readFileSync(join(SRC, "server.ts"), "utf8");
const uiCss = readFileSync(join(SRC, "ui.css"), "utf8");

const prov = {
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
  aa: "fixtures",
} as never;

describe("page shell (Phase C)", () => {
  const html = renderSearch("", [], new Map(), prov);

  test("links the built stylesheet and carries no inline CSS const", () => {
    expect(html).toContain('<link rel="stylesheet" href="/assets/app.css">');
    expect(html).not.toContain("background: #0e1117; color: #e6edf3; }");
    expect(renderSource).not.toContain("const CSS");
  });

  test("dark is the default theme, with a pre-CSS bootstrap for the stored choice", () => {
    expect(html).toContain('<html lang="en" data-theme="dark">');
    expect(html).toContain("localStorage.getItem('pm-theme')");
    // The bootstrap must run before the stylesheet link so there is no flash.
    expect(html.indexOf("pm-theme")).toBeLessThan(html.indexOf("/assets/app.css"));
  });

  test("the theme toggle is Datastar signals/effect/click, persisting to localStorage", () => {
    expect(html).toContain('data-signals="{ pmTheme:');
    expect(html).toContain('data-init="$pmTheme = document.documentElement');
    expect(html).toContain("data-effect=\"document.documentElement.setAttribute('data-theme', $pmTheme)");
    expect(html).toContain("localStorage.setItem('pm-theme', $pmTheme)");
    expect(html).toContain('data-on:click="$pmTheme = $pmTheme ===');
    expect(html).toContain('class="btn btn-ghost btn-sm theme-toggle"');
  });

  test("the pages dropdown navigates through Datastar, not an inline onchange", () => {
    expect(html).toContain('class="select select-bordered select-sm"');
    expect(html).toContain('data-on:change="if (evt.target.value)');
    expect(html).not.toContain('onchange="if(this.value)');
  });

  test("the navbar is a DaisyUI navbar and the search controls use btn/input classes", () => {
    expect(html).toContain('<nav class="navbar bg-base-200');
    expect(html).toContain('class="btn btn-sm">Search</button>');
    expect(html).toContain('class="input input-bordered input-sm"');
  });
});

describe("stylesheet pipeline (Phase C)", () => {
  test("src/ui.css is Tailwind v4 + DaisyUI with dark default and light available", () => {
    expect(uiCss).toContain('@import "tailwindcss"');
    expect(uiCss).toContain('@plugin "daisyui"');
    expect(uiCss).toContain("dark --default");
    expect(uiCss).toContain("light");
  });

  test("the built asset is committed and contains the DaisyUI components in use", () => {
    const built = join(SRC, "assets", "app.css");
    expect(existsSync(built)).toBe(true);
    const css = readFileSync(built, "utf8");
    expect(css.length).toBeGreaterThan(50_000);
    for (const marker of [".btn", ".badge", ".card", ".navbar", ".select", ".table-zebra", "--color-primary"]) {
      // The explicit-theme scoped rules must survive the Tailwind build
      // (a malformed generated selector once dropped a span of them).
      for (const scoped of [
        'html[data-theme="light"] .skel',
        'html[data-theme="light"] nav select',
        'html[data-theme="light"] .cform textarea',
      ]) {
        expect(css).toContain(scoped);
      }
      expect(css).toContain(marker);
    }
  });

  test("the legacy rules the suite pins moved with the stylesheet, verbatim key lines", () => {
    expect(uiCss).toContain(
      "body { margin: 0; font: 14px/1.45 system-ui, -apple-system, sans-serif; background: #0e1117; color: #e6edf3; }",
    );
    expect(uiCss).toContain("@media (prefers-color-scheme: light)");
  });
});

describe("server (Phase C)", () => {
  test("serves /assets/app.css as text/css", () => {
    expect(serverSource).toContain('path === "/assets/app.css"');
    expect(serverSource).toContain('"content-type": "text/css; charset=utf-8"');
  });

  test("Bun.serve carries the 60s idleTimeout for slow fragments (Phase B §13.1)", () => {
    expect(serverSource).toContain("idleTimeout: 60");
  });
});
