import { describe, expect, test } from "bun:test";
import seed from "../../grist/seed/roadmap.json" with { type: "json" };
import { loadConfig } from "../src/config.ts";
import { chicagoYmd } from "../src/derive.ts";
import { FIXTURE_ROADMAP, fixtureSource } from "../src/sources/fixtures.ts";
import { mapGristRoadmapRecord, liveSource } from "../src/sources/live.ts";
import { renderRoadmap } from "../src/render.ts";
import type { RoadmapRow } from "../src/model.ts";
import { validateRoadmapHorizon } from "../src/roadmap-horizon.ts";

const row = (id: number, title: string, horizon: string, area: string, status: string): RoadmapRow => ({
  id,
  subject: `grist:doc/Roadmap/${id}`,
  title,
  area,
  status,
  horizon,
  source: "",
  notes: "",
  links: [],
});

describe("roadmap Horizon contract (#1207)", () => {
  test("accepts canonical grammar and normalizes only for classification", () => {
    expect(validateRoadmapHorizon("2026-10-24").valid).toBe(true);
    expect(validateRoadmapHorizon("Q4 2026").valid).toBe(true);
    expect(validateRoadmapHorizon(" q1 2027 ")).toEqual({ valid: true, canonical: "Q1 2027", changed: true });
    expect(validateRoadmapHorizon("nOw")).toEqual({ valid: true, canonical: "Now", changed: true });
    expect(validateRoadmapHorizon("Unscheduled").valid).toBe(true);
  });

  test("rejects malformed values and impossible calendar dates", () => {
    for (const value of ["", "Q5 2027", "2026-02-30", "2026-10-24 extra", "Soonish"]) {
      expect(validateRoadmapHorizon(value).valid).toBe(false);
    }
  });

  test("validates every seed horizon and maps invalid live values without dropping them", () => {
    const invalidSeed = seed.rows.filter((r) => !validateRoadmapHorizon(r.Horizon).valid);
    expect(invalidSeed).toEqual([]);
    expect(FIXTURE_ROADMAP.every((r) => !r.horizonError)).toBe(true);

    const live = mapGristRoadmapRecord("doc", {
      id: 9,
      fields: { Title: "Bad live value", Area: "PM stack", Status: "Planned", Horizon: "Q5 2026", Source: "", Notes: "" },
    });
    expect(live.title).toBe("Bad live value");
    expect(live.horizon).toBe("Q5 2026");
    expect(live.horizonError).toContain("Unrecognized Horizon value");
  });

  test("keeps all rows visible and shows consistent theme rollups at day and quarter grains", async () => {
    const today = chicagoYmd();
    const rows = [
      row(1, "Today done", today, "Alpha", "Done"),
      row(2, "Outside planned", "2000-01-01", "Alpha", "Planned"),
      row(3, "Quarter idea", "Q4 2026", "Beta", "Idea"),
      { ...row(4, "Invalid horizon", "Soonish", "Beta", "Planned"), horizonError: "Unrecognized Horizon value." },
    ];
    const source = fixtureSource();
    const items = await source.items([]);
    const day = renderRoadmap(rows, items, source.provenance(), "today");
    const quarter = renderRoadmap(rows, items, source.provenance(), "quarter");

    for (const title of ["Today done", "Outside planned", "Quarter idea", "Invalid horizon"]) {
      expect(day).toContain(title);
      expect(quarter).toContain(title);
    }
    expect(day).toContain("Outside this window (1)");
    expect(day).toContain("Undated or coarse horizons (2)");
    expect(day).toContain("Alpha <span class=\"muted theme-rollup\">(1/2 done)</span>");
    expect(day).toContain("Beta <span class=\"muted theme-rollup\">(0/2 done)</span>");
    expect(quarter).toContain("(1/2 done)");
    expect(day).toContain("Invalid Horizon");
    expect(day).toContain("Original values are preserved");
  });

  test("unavailable live Grist remains explicitly reported while fixtures stay available", async () => {
    const source = liveSource(loadConfig({ ...process.env, GRIST_API_KEY: "", PM_PROJECTOR_URL: "off" }));
    const rows = await source.roadmap();
    expect(rows.length).toBeGreaterThan(0);
    expect(source.provenance().roadmap).toContain("grist unreachable");
  });
});
