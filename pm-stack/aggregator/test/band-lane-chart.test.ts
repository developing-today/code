// Behavioural tests for the Cost Explorer band lane charts (#8600).
//
// aa.test.ts historically pinned the lane chart only by asserting the
// emitted page CONTAINS the renderer's source strings — the renderer
// never executed against data anywhere. Here the real emitted
// explorer script is taken from renderAa's page, the renderer's own
// function definitions are extracted from it verbatim (no copies),
// and bandLaneChart runs against known band data with assertions on
// the SVG it produces.
import { describe, expect, test } from "bun:test";
import { renderAa } from "../src/render.ts";
import { FIXTURE_AA } from "../src/sources/fixtures.ts";
import type { Provenance } from "../src/model.ts";

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
  aa: "fixtures",
};

/** The emitted explorer script body that defines bandLaneChart. */
function explorerScript(): string {
  const html = renderAa(FIXTURE_AA, PROV);
  const scripts = [...html.matchAll(/<script(?:\s[^>]*)?>([\s\S]*?)<\/script>/g)].map((m) => m[1]);
  const body = scripts.find((b) => b.includes("function bandLaneChart"));
  if (!body) throw new Error("emitted page has no script defining bandLaneChart");
  return body;
}

/** Slice [startMarker, endMarker) out of the emitted script; both must exist. */
function sliceFn(script: string, startMarker: string, endMarker: string): string {
  const a = script.indexOf(startMarker);
  const b = script.indexOf(endMarker);
  if (a < 0 || b < 0 || b <= a) throw new Error(`cannot extract ${startMarker} .. ${endMarker}`);
  return script.slice(a, b);
}

type BandPoint = { key: string; family: string; effort?: string; cost?: number | null; score?: number | null; time?: number | null };
type Band = { label: string; members: BandPoint[]; choices: BandPoint[]; leader?: BandPoint };
type BandLaneChart = (title: string, metric: string, axisLabel: string, bands: Band[]) => string;

function loadBandLaneChart(): BandLaneChart {
  const script = explorerScript();
  const helpers = sliceFn(script, "function esc(s)", "function fmtCtx(");
  const labels = sliceFn(script, "var ESHORT", "var goCapByName");
  const chart = sliceFn(script, "function bandLaneChart(", "function bandGroup(");
  // famColor is populated from the page's point data at runtime; the
  // chart falls back to a default fill for unmapped families, and the
  // assertions below pin geometry/styling roles, not palette values.
  const factory = new Function(`"use strict";\nvar famColor = {};\n${labels}\n${helpers}\n${chart}\nreturn bandLaneChart;`);
  return factory() as BandLaneChart;
}

const bandLaneChart = loadBandLaneChart();

const pt = (key: string, cost: number | null): BandPoint => ({ key, family: "Fam" + key, effort: "low", cost });

function circles(svg: string): string[] {
  return [...svg.matchAll(/<circle class="xpt"[^>]*>/g)].map((m) => m[0]);
}

describe("bandLaneChart behavioural (#8600)", () => {
  test("axis range comes from members; plotted points come from choices", () => {
    const bands: Band[] = [
      {
        label: "Band A",
        // m3 is a member only: it must stretch the axis to $500 but
        // must NOT be plotted.
        members: [pt("m1", 0.5), pt("m2", 5), pt("m3", 500)],
        choices: [pt("cheap", 0.5), pt("mid", 5)],
      },
    ];
    const svg = bandLaneChart("T", "cost", "USD / task (lower is cheaper)", bands);
    expect(svg).toContain('class="xpl-band-lanes"');
    expect(svg).toContain("(log scale)");
    // Two choices plotted, not three members.
    expect(circles(svg)).toHaveLength(2);
    // The axis reaches the member-only maximum: the last tick is $500.
    expect(svg).toContain(">$500.00<");
    // Log geometry: values 0.5 / 5 / 500 are log-spaced 0 / 1/3 / 1 of
    // the 512-wide plot starting at x=232.
    expect(svg).toContain('cx="232.0"'); // cheap at lo
    expect(svg).toContain('cx="402.7"'); // mid at 232 + 512/3
    // Lane geometry (the pointY-vs-cy regression the old string
    // assertions guarded): lane 0 centre is cy=43, point jitter is
    // ((pi % 5) - 2) * 2.5, so the points land at 38.0 and 40.5.
    expect(svg).toContain('cy="38.0"');
    expect(svg).toContain('cy="40.5"');
  });

  test("the band leader is styled as leader; other choices as near-leaders", () => {
    const bands: Band[] = [{ label: "Band A", members: [pt("cheap", 0.5), pt("mid", 5)], choices: [pt("cheap", 0.5), pt("mid", 5)] }];
    const svg = bandLaneChart("T", "cost", "USD / task", bands);
    const [leader, other] = circles(svg);
    // No explicit band.leader: choices[0] leads.
    expect(leader).toContain('r="5"');
    expect(leader).toContain('stroke="#fff"');
    expect(leader).toContain('fill-opacity="0.95"');
    expect(other).toContain('r="3.8"');
    expect(other).toContain('fill-opacity="0.65"');
    expect(svg).toContain("— band leader");
    expect(svg).toContain("— near-leader");
    // An explicit leader by key wins over position.
    const flipped: Band[] = [
      { label: "Band A", members: [pt("cheap", 0.5), pt("mid", 5)], choices: [pt("cheap", 0.5), pt("mid", 5)], leader: pt("mid", 5) },
    ];
    const svg2 = bandLaneChart("T", "cost", "USD / task", flipped);
    const [first, second] = circles(svg2);
    expect(first).toContain('r="3.8"');
    expect(second).toContain('r="5"');
  });

  test("lo === hi widens: a single cost value lands at the log centre", () => {
    const bands: Band[] = [{ label: "Solo", members: [pt("only", 10)], choices: [pt("only", 10)] }];
    const svg = bandLaneChart("T", "cost", "USD / task", bands);
    // Widened to [10/1.4, 10*1.4]; log-midpoint is 10 itself, which is
    // the plot centre: 232 + 512/2 = 488.
    expect(svg).toContain('cx="488.0"');
    expect(circles(svg)).toHaveLength(1);
  });

  test("lo === hi widens for score too, on the linear scale", () => {
    const p: BandPoint = { key: "s1", family: "Fams1", effort: "low", score: 80 };
    const svg = bandLaneChart("T", "score", "Score", [{ label: "Solo", members: [p], choices: [p] }]);
    expect(svg).toContain("(linear scale)");
    // Widened to [79, 81]; 80 is the centre.
    expect(svg).toContain('cx="488.0"');
  });

  test("no usable member values renders nothing", () => {
    const bands: Band[] = [{ label: "Empty", members: [pt("a", null)], choices: [pt("a", null)] }];
    expect(bandLaneChart("T", "cost", "USD / task", bands)).toBe("");
    // Bands without choices are dropped before the axis is computed.
    const noChoices: Band[] = [{ label: "Empty", members: [pt("a", 3)], choices: [] }];
    expect(bandLaneChart("T", "cost", "USD / task", noChoices)).toBe("");
  });
});
