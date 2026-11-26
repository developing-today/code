import { Packr } from "msgpackr";
import { describe, expect, it } from "vitest";
import { decodeTaggedViewPatch, decodeViewPatch } from "./screen-codec";
import {
  applyScreenPatchV2,
  isValidSemanticTag,
  isValidScreenSurfaceV2,
  resolveTaggedCell,
  taggedRenderData,
  type FallbackPlanData,
  type ScreenSurfaceV2,
  type TaggedScreenCell,
} from "./world-surface";

/**
 * Screen v2 tagged cells (#14731 slice 5). Oracle: the id-core rules —
 * Screen v1 stays strict and unchanged, tags exist only in the v2
 * representation (namespaced `sys.*`/`world.*`, sorted and unique),
 * patch application validates tags, and the four-rung resolver
 * connected to a tagged cell passes the blue-cape-with-sparkles
 * example at every rung with a non-blank glyph and deterministic
 * dropped-tag/filter diagnostics.
 */

const BLUE_CAPE_TAGS = ["sys.color.blue", "sys.effect.sparkle", "world.appearance.cape"];

function cell(symbol: string, tags: string[]): TaggedScreenCell {
  return { symbol, fg: 0, bg: 0, bold: false, columns: 1, token: null, tags };
}

function blueCapeSurface(): ScreenSurfaceV2 {
  return {
    kind: "screen",
    version: 2,
    width: 2,
    height: 1,
    cells: [cell("C", [...BLUE_CAPE_TAGS]), cell(" ", [])],
  };
}

function blueCapePlan(): FallbackPlanData {
  return {
    rich: { kind: "asset", ref: "hero@1", retainedTags: [...BLUE_CAPE_TAGS], retainedFilters: ["fx/sparkle@1"] },
    static: { kind: "asset", ref: "hero-static@1", retainedTags: [...BLUE_CAPE_TAGS], retainedFilters: [] },
    tileset: {
      kind: "asset",
      ref: "tiles.blue-cape@1",
      retainedTags: ["sys.color.blue", "world.appearance.cape"],
      retainedFilters: [],
    },
    glyph: { kind: "glyph", glyph: "C", retainedTags: [], retainedFilters: [] },
  };
}

describe("semantic tags", () => {
  it("accepts namespaced tags and rejects the rest", () => {
    for (const good of ["sys.color.blue", "world.appearance.cape", "sys.effect.sparkle"]) {
      expect(isValidSemanticTag(good)).toBe(true);
    }
    for (const bad of ["", "blue", "sys", "other.color.blue", "sys.Color.blue", "sys..blue", "sys.1blue"]) {
      expect(isValidSemanticTag(bad)).toBe(false);
    }
  });

  it("validates a v2 surface and rejects v1-shaped or unsorted-tag surfaces", () => {
    expect(isValidScreenSurfaceV2(blueCapeSurface())).toBe(true);
    expect(isValidScreenSurfaceV2({ ...blueCapeSurface(), version: 1 })).toBe(false);
    const unsorted = blueCapeSurface();
    unsorted.cells[0] = cell("C", ["world.appearance.cape", "sys.color.blue"]);
    expect(isValidScreenSurfaceV2(unsorted)).toBe(false);
  });
});

describe("Screen v2 patches", () => {
  it("applies a patch preserving tags and rejects invalid tag sets", () => {
    const before = blueCapeSurface();
    const applied = applyScreenPatchV2(before, {
      version: 2,
      width: 2,
      height: 1,
      runs: [{ start: 1, cells: [cell("x", ["sys.color.blue"])] }],
    });
    expect(applied?.cells[1]).toEqual(cell("x", ["sys.color.blue"]));
    expect(applied?.cells[0].tags).toEqual(BLUE_CAPE_TAGS);
    expect(
      applyScreenPatchV2(before, {
        version: 2,
        width: 2,
        height: 1,
        runs: [{ start: 1, cells: [cell("x", ["unnamespaced"])] }],
      }),
    ).toBeNull();
    // A v1 patch does not apply to a v2 surface.
    expect(
      applyScreenPatchV2(
        before,
        // @ts-expect-error v1 patch shape is not a v2 patch
        { version: 1, width: 2, height: 1, runs: [] },
      ),
    ).toBeNull();
  });
});

describe("fallback resolver on tagged cells", () => {
  it("passes the blue-cape example at every rung with deterministic diagnostics", () => {
    const surface = blueCapeSurface();
    const plan = blueCapePlan();
    const filters = ["fx/sparkle@1"];
    const expected = [
      { start: "rich", rung: "rich", droppedTags: [], droppedFilters: 0 },
      { start: "static", rung: "static", droppedTags: [], droppedFilters: 1 },
      { start: "tileset", rung: "tileset", droppedTags: ["sys.effect.sparkle"], droppedFilters: 1 },
      { start: "glyph", rung: "glyph", droppedTags: [...BLUE_CAPE_TAGS], droppedFilters: 1 },
    ] as const;
    for (const { start, rung, droppedTags, droppedFilters } of expected) {
      const resolved = resolveTaggedCell(surface.cells[0], 0, plan, start, filters);
      expect(resolved?.rung).toBe(rung);
      expect(resolved?.symbol).toBe("C");
      expect(resolved?.fallbackGlyph).toBe("C");
      expect(resolved?.fallbackGlyph.trim()).not.toBe("");
      expect(resolved?.fallbackGlyph).not.toBe("?");
      expect(resolved?.droppedTags).toEqual([...droppedTags]);
      expect(resolved?.droppedFilters).toHaveLength(droppedFilters);
    }
    const data = taggedRenderData(surface, new Map([[0, plan]]), "tileset", filters);
    expect(data).toHaveLength(1);
    expect(data?.[0].droppedTags).toEqual(["sys.effect.sparkle"]);
  });

  it("refuses a plan that claims tags the cell does not carry", () => {
    const surface = blueCapeSurface();
    const plan = blueCapePlan();
    plan.rich = {
      kind: "asset",
      ref: "hero@1",
      retainedTags: [...BLUE_CAPE_TAGS, "world.appearance.hood"],
      retainedFilters: [],
    };
    expect(resolveTaggedCell(surface.cells[0], 0, plan, "rich", [])).toBeNull();
  });
});

describe("decodeTaggedViewPatch", () => {
  const packr = new Packr({ useRecords: false });
  function frameBytes(patch: unknown): Uint8Array {
    return packr.pack({
      version: 1,
      frame: { sequence: 7, view_revision: 3, base_view_revision: 2, resume_token: null, patch },
    });
  }

  it("decodes a tagged patch and validates its tags", () => {
    const bytes = frameBytes({
      version: 2,
      width: 2,
      height: 1,
      runs: [{ start: 0, cells: [cell("C", [...BLUE_CAPE_TAGS])] }],
    });
    const decoded = decodeTaggedViewPatch(bytes);
    expect(decoded?.patch.version).toBe(2);
    expect(decoded?.patch.runs[0].cells[0].tags).toEqual(BLUE_CAPE_TAGS);
    // The v1 decoder rejects the same frame instead of dropping tags.
    expect(decodeViewPatch(bytes)).toBeNull();
  });

  it("rejects v1 patches and invalid tag sets", () => {
    expect(decodeTaggedViewPatch(frameBytes({ version: 1, width: 2, height: 1, runs: [] }))).toBeNull();
    expect(
      decodeTaggedViewPatch(
        frameBytes({
          version: 2,
          width: 2,
          height: 1,
          runs: [{ start: 0, cells: [cell("C", ["unnamespaced"])] }],
        }),
      ),
    ).toBeNull();
  });
});
