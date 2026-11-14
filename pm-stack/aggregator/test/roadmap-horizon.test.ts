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
      fields: {
        Title: "Bad live value",
        Area: "PM stack",
        Status: "Planned",
        Horizon: "Q5 2026",
        Source: "",
        Notes: "",
      },
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
    expect(day).toContain('Alpha <span class="muted theme-rollup">(1/2 done)</span>');
    expect(day).toContain('Beta <span class="muted theme-rollup">(0/2 done)</span>');
    expect(quarter).toContain("(1/2 done)");
    expect(day).toContain("Invalid Horizon");
    expect(day).toContain("Original values are preserved");
  });

  test("unavailable live Grist remains explicitly reported while fixtures stay available", async () => {
    const source = liveSource(
      loadConfig({ ...process.env, PM_AGG_SOURCE: "live", GRIST_API_KEY: "", PM_PROJECTOR_URL: "off" }),
    );
    const rows = await source.roadmap();
    expect(rows.length).toBeGreaterThan(0);
    expect(source.provenance().roadmap).toContain("grist unreachable");
  });

  describe("property-based Horizon parser coverage", () => {
    // Oracle: an independent Gregorian calendar model - JS Date UTC
    // round-trip - decides date validity. The parser under test uses an
    // arithmetic leap-year table instead, so agreement across generated
    // inputs is a real cross-check, not a restatement of the parser.
    const mulberry32 = (seedStart: number) => {
      let a = seedStart >>> 0;
      return () => {
        a = (a + 0x6d2b79f5) | 0;
        let t = Math.imul(a ^ (a >>> 15), 1 | a);
        t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
        return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
      };
    };
    const pad = (n: number, w: number) => String(n).padStart(w, "0");
    const oracleValidDate = (year: number, month: number, day: number): boolean => {
      const dt = new Date(0);
      dt.setUTCFullYear(year, month - 1, day);
      return dt.getUTCFullYear() === year && dt.getUTCMonth() === month - 1 && dt.getUTCDate() === day;
    };

    test("generated dates: validity always agrees with the calendar oracle", () => {
      const rand = mulberry32(0x1207);
      const edgeYears = [1900, 2000, 2100, 2400, 2024, 2026];
      for (let i = 0; i < 900; i++) {
        const year = i % 3 === 0 ? edgeYears[Math.floor(rand() * edgeYears.length)] : 1800 + Math.floor(rand() * 800);
        const month = Math.floor(rand() * 15); // 0..14 - includes invalid months
        const day = Math.floor(rand() * 34); // 0..33 - includes invalid days
        const value = `${pad(year, 4)}-${pad(month, 2)}-${pad(day, 2)}`;
        const result = validateRoadmapHorizon(value);
        expect(result.valid).toBe(oracleValidDate(year, month, day));
        if (result.valid) expect(result).toEqual({ valid: true, canonical: value, changed: false });
      }
    });

    test("generated valid values: canonicalization is idempotent under case/whitespace variation", () => {
      const rand = mulberry32(0x3958);
      const buckets = ["Now", "Next", "Later", "Ongoing", "Unscheduled"];
      const flipCase = (s: string) => [...s].map((c) => (rand() < 0.5 ? c.toUpperCase() : c.toLowerCase())).join("");
      const wrap = (s: string) => `${" ".repeat(Math.floor(rand() * 3))}${s}${"\t".repeat(Math.floor(rand() * 2))}`;
      for (let i = 0; i < 600; i++) {
        const kind = Math.floor(rand() * 3);
        let canonical: string;
        let variant: string;
        if (kind === 0) {
          let year = 0,
            month = 0,
            day = 0;
          do {
            year = 1800 + Math.floor(rand() * 800);
            month = 1 + Math.floor(rand() * 12);
            day = 1 + Math.floor(rand() * 31);
          } while (!oracleValidDate(year, month, day));
          canonical = `${pad(year, 4)}-${pad(month, 2)}-${pad(day, 2)}`;
          variant = wrap(canonical);
        } else if (kind === 1) {
          const q = 1 + Math.floor(rand() * 4);
          const year = 1800 + Math.floor(rand() * 800);
          canonical = `Q${q} ${year}`;
          variant = wrap(`${flipCase(`Q${q}`)}${" ".repeat(1 + Math.floor(rand() * 3))}${year}`);
        } else {
          canonical = buckets[Math.floor(rand() * buckets.length)];
          variant = wrap(flipCase(canonical));
        }
        const result = validateRoadmapHorizon(variant);
        expect(result).toEqual({ valid: true, canonical, changed: variant !== canonical });
        // Idempotence: re-validating the canonical form is a fixed point.
        expect(validateRoadmapHorizon(canonical)).toEqual({ valid: true, canonical, changed: false });
      }
    });

    test("generated malformed values are rejected", () => {
      const rand = mulberry32(0x7297);
      const buckets = ["Now", "Next", "Later", "Ongoing", "Unscheduled"];
      for (let i = 0; i < 300; i++) {
        const year = 1800 + Math.floor(rand() * 800);
        const bad = [
          `Q${rand() < 0.5 ? 0 : 5 + Math.floor(rand() * 5)} ${year}`,
          `${year}/${pad(1 + Math.floor(rand() * 12), 2)}/${pad(1 + Math.floor(rand() * 28), 2)}`,
          `${year}-${pad(1 + Math.floor(rand() * 12), 2)}`,
          `${buckets[Math.floor(rand() * buckets.length)]}!`,
          `Q1 ${pad(year, 5)}`,
          `${pad(year, 4)}-${1 + Math.floor(rand() * 9)}-${pad(1 + Math.floor(rand() * 28), 2)}`,
        ][Math.floor(rand() * 6)];
        expect(validateRoadmapHorizon(bad).valid).toBe(false);
      }
      for (const nonText of [null, undefined, 20261024, {}, []]) {
        expect(validateRoadmapHorizon(nonText).valid).toBe(false);
      }
    });
  });
});
