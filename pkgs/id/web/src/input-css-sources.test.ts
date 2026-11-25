import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));

/** The @source paths declared in input.css, resolved against its directory. */
function declaredSources(): string[] {
  const css = readFileSync(resolve(here, "input.css"), "utf8");
  const paths: string[] = [];
  for (const match of css.matchAll(/@source\s+"([^"]+)"/g)) {
    paths.push(resolve(here, match[1] as string));
  }
  return paths;
}

describe("input.css @source directives", () => {
  // Oracle: #15170 — a @source path that names no file is silently
  // ignored by the Tailwind build, so every utility class used only in
  // that file vanishes from styles.css. The Rust templates' copy moved
  // to crates/id-web in the crate split while input.css kept pointing
  // at the old src/web path; the world view pane lost min-h-32 (and
  // its other template-only classes), collapsed to one text line, and
  // the client's viewport measurement reported height 1, so the host
  // cropped every world surface to a single row.
  it("every declared @source path resolves to an existing file", () => {
    const sources = declaredSources();
    expect(sources.length).toBeGreaterThan(0);
    for (const source of sources) {
      expect(existsSync(source), `@source ${source} must exist`).toBe(true);
    }
  });

  it("scans the Rust templates that carry the page markup", () => {
    const templates = resolve(here, "../../crates/id-web/src/templates.rs");
    expect(declaredSources()).toContain(templates);
    // The world view pane's sizing classes live only in the templates:
    // if they are not scanned, the pane has no minimum height.
    const markup = readFileSync(templates, "utf8");
    expect(markup).toContain("min-h-32");
  });
});
