// AA cost explorer: the collector's pure pieces (Test_Runs mapping,
// opencode-config coverage inputs, name matching, payload assembly,
// feed-section shape), the page manifest entry, and the fixture
// render + HTTP routes. Follows test/slice3.test.ts / program.test.ts.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { spawn, type Subprocess } from "bun";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  aaAbsent,
  aaCaiPoints,
  aaCatalogFromConfig,
  aaPoints,
  caiFamily,
  aaSectionFromSupabase,
  computeAaCoverage,
  isAaSection,
  normalizeModelName,
  parseOpencodeCoverageInputs,
  stripJsonComments,
  summarizeAa,
  testRunsFromSheetRows,
  type AaRawPayload,
} from "../src/aa.ts";
import { PAGES } from "../src/pages.ts";
import { renderAa } from "../src/render.ts";
import { FIXTURE_AA, fixtureSource } from "../src/sources/fixtures.ts";
import type { AaCaiRow, AaPrice, AaRanking, AaSection, AaTestRun, Provenance } from "../src/model.ts";

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

describe("stripJsonComments", () => {
  test("removes // and /* */ comments but keeps URLs inside strings", () => {
    const out = stripJsonComments(`{ "npm": "@ai-sdk/openai-compatible", // provider pkg
      "options": { "baseURL": "https://api.example.com/v1" }, /* block */
      "note": "a // b" }`);
    const parsed = JSON.parse(out) as { options: { baseURL: string }; note: string };
    expect(parsed.options.baseURL).toBe("https://api.example.com/v1");
    expect(parsed.note).toBe("a // b");
  });
});

// A snippet shaped like the repo config: JSONC comments, a nested
// route (scout wraps speed), an object target with a variant remap,
// and an instanceOf alias provider with no inline models.
const CONFIG_SNIPPET = `{
  // comment about the config
  "provider": {
    "openai": { "models": { "gpt-6-luna": {}, "gpt-6.1-sol": {} } },
    "commandcode": { "models": { "reflection/beam:free": {}, "deepseek/deepseek-v4-pro": {} } },
    "opencode-go.personal": { "instanceOf": "opencode-go" },
  },
  "experimental": {
    "model_routes": {
      "speed": { "targets": ["openai/gpt-6-luna#max"] },
      "scout": { "targets": ["opencode-route/speed", { "model": "deepseek/deepseek-v4-flash#low", "variants": { "xhigh": "max" } }] },
    },
  },
}`;

describe("opencode coverage inputs", () => {
  test("catalog = model ids (sans instanceOf aliases); pooled expands nested routes and strips #pins", () => {
    const inputs = parseOpencodeCoverageInputs(CONFIG_SNIPPET);
    expect(inputs).not.toBeNull();
    expect(inputs!.catalog).toContain("openai/gpt-6-luna");
    expect(inputs!.catalog).toContain("commandcode/reflection/beam:free");
    expect(inputs!.catalog.some((id) => id.startsWith("opencode-go.personal"))).toBe(false);
    expect(inputs!.pooled).toContain("openai/gpt-6-luna");
    expect(inputs!.pooled).toContain("deepseek/deepseek-v4-flash");
    expect(inputs!.pooled.some((id) => id.includes("#"))).toBe(false);
  });

  test("null on unparseable config", () => {
    expect(parseOpencodeCoverageInputs("{ not json")).toBeNull();
  });
});

describe("Test_Runs mapping", () => {
  // Header order deliberately shuffled vs the real sheet: mapping
  // must key off the header row, not positions.
  const rows = [
    {
      rowIndex: 1,
      cells: [
        "time_per_task_seconds",
        "test_run_key",
        "raw_score",
        "ttft_seconds",
        "snapshot_date",
        "model_variant_key",
        "cost_per_task_usd",
        "first_answer_seconds",
        "e2e_500_seconds",
        "output_speed_tps",
      ],
    },
    {
      rowIndex: 2,
      cells: [
        "407.3",
        "openai|luna|k",
        38,
        "93.76",
        "2026-09-28",
        "openai|gpt-6-luna|max",
        "0.07",
        "93.76",
        "97.71",
        "145",
      ],
    },
    {
      rowIndex: 3,
      cells: ["", "deepseek|flash|k", "", "", "2026-09-28", "deepseek|v4.1-flash|max", "0.49", "", "", "231.2"],
    },
    { rowIndex: 4, cells: ["123", "", "", "", "", "", "", "", "", ""] }, // no key -> dropped
  ];
  test("maps by header; empty cells become null (never invented)", () => {
    const runs = testRunsFromSheetRows(rows);
    expect(runs.length).toBe(2);
    const luna = runs.find((r) => r.testRunKey === "openai|luna|k")!;
    const flash = runs.find((r) => r.testRunKey === "deepseek|flash|k")!;
    expect(luna.ttftSeconds).toBe(93.76);
    expect(luna.e2e500Seconds).toBe(97.71);
    expect(luna.timePerTaskSeconds).toBe(407.3);
    expect(luna.intelligence).toBe(38);
    expect(flash.ttftSeconds).toBeNull();
    expect(flash.timePerTaskSeconds).toBeNull();
    expect(flash.costPerTaskUsd).toBe(0.49);
  });
});

describe("coverage name matching", () => {
  test("normalization keeps point versions distinct (GPT-6 vs GPT-6.1)", () => {
    expect(normalizeModelName("GPT-6 Sol")).toBe("gpt 6 sol");
    expect(normalizeModelName("gpt-6.1-sol")).toBe("gpt 6 1 sol");
    expect(normalizeModelName("GPT-6 Sol")).not.toBe(normalizeModelName("gpt-6.1-sol"));
  });

  test("Beam: catalogued, unmeasured, unpooled — lands in the unmeasured list, not the measured one", () => {
    const cov = computeAaCoverage(["GPT-6 Luna", "Kimi K3", "DeepSeek V4 Pro 0813"], {
      catalog: ["openai/gpt-6-luna", "commandcode/reflection/beam:free", "commandcode/deepseek/deepseek-v4-pro"],
      pooled: ["openai/gpt-6-luna"],
    });
    expect(cov.measuredNotPooled).toContain("Kimi K3");
    expect(cov.measuredNotPooled).toContain("DeepSeek V4 Pro 0813"); // via the 0813 alias
    expect(cov.measuredNotPooled).not.toContain("GPT-6 Luna"); // pooled
    expect(cov.catalogNotPooledMeasured).toContain("DeepSeek V4 Pro 0813");
    expect(cov.catalogNotPooledMeasured).not.toContain("Kimi K3");
    expect(cov.measuredNotInCatalog).toContain("Kimi K3");
    expect(cov.catalogNotPooledUnmeasured).toContain("commandcode/reflection/beam:free");
  });
});

function rawPayload(): AaRawPayload {
  return {
    batch: "8152e746-4805-4a11-9664-317b44ca9f1b",
    sources: [
      {
        name: "model_value_data.csv",
        kind: "drive-csv",
        importedAt: "2026-10-26T13:16:45Z",
        driveModifiedAt: "2026-10-19T06:55:29Z",
      },
    ],
    rankings: [
      {
        rowIndex: "3",
        testRunKey: "openai|luna|k",
        modelVariantKey: "openai|gpt-6-luna|max",
        family: "GPT-6 Luna",
        creator: "OpenAI",
        effort: "max",
        benchmarkVersionKey: "aa-v4.3.2",
        rawScore: "38",
        adjustedScore: "38",
        costTask: "0.07",
        timeTaskSec: "407.3",
        speedTps: "145",
        qualStatus: "Contender",
        availability: "yes",
        eligible: true,
        intelEffPerUsd: "28.57",
        usdPerUsableUnit: "0.035",
        pointsPerDollar: "542.9",
        pointsPerSecond: "0.093",
        balancedValue: "0.41",
        smartValue: "0.44",
        speedValue: "0.31",
        rankBalanced: "24",
        rankSmart: "22",
        rankSpeed: "31",
        rankIntelligence: "30",
        rankLowCost: "4",
      },
    ],
    thresholds: [
      {
        testRunKey: "openai|luna|k",
        target: "30",
        usable: true,
        levelledScore: "38",
        costTask: "0.07",
        timeMin: "6.79",
        usdPerUsablePoint: "0.009",
      },
      {
        testRunKey: "openai|luna|k",
        target: "40",
        usable: false,
        levelledScore: "38",
        costTask: "0.07",
        timeMin: "6.79",
        usdPerUsablePoint: null,
      },
    ],
    configs: [
      {
        modelName: "GPT-6 Luna",
        benchmark: "AA Intelligence Index",
        effort: "Reasoning, Max Effort",
        intelligence: "38",
        costPerTaskUsd: "0.07",
        speedTps: "145.2",
      },
    ],
    testRunRows: [
      { rowIndex: 1, cells: ["test_run_key", "ttft_seconds"] },
      { rowIndex: 2, cells: ["openai|luna|k", "93.76"] },
    ],
  };
}

describe("payload assembly + section shape", () => {
  test("string numerics from the Management API are coerced; coverage computed when config parses", () => {
    const section = aaSectionFromSupabase(rawPayload(), CONFIG_SNIPPET, "2026-10-26T18:05:00Z");
    expect(section).not.toBeNull();
    expect(section!.status).toBe("ok");
    expect(section!.rankings[0].costTask).toBe(0.07);
    expect(section!.rankings[0].rankBalanced).toBe(24);
    expect(section!.testRuns[0].ttftSeconds).toBe(93.76);
    expect(section!.coverage?.catalogNotPooledUnmeasured).toContain("commandcode/reflection/beam:free");
    expect(section!.coverage?.measuredNotPooled).toEqual([]); // Luna is pooled in the snippet
    expect(isAaSection(section)).toBe(true);
  });

  test("the same config read also builds the Model Explorer catalog", () => {
    const section = aaSectionFromSupabase(rawPayload(), CONFIG_SNIPPET, "2026-10-26T18:05:00Z");
    const catalog = section!.catalog;
    expect(catalog).toBeDefined();
    const luna = catalog!.find((e) => e.id === "openai/gpt-6-luna")!;
    expect(luna.routes).toEqual(["scout", "speed"]); // nested route expanded, #pin stripped
    expect(luna.aaScore).toBe(38); // matched to the payload's config row
    expect(luna.aaCostPerTask).toBe(0.07);
    const beam = catalog!.find((e) => e.id === "commandcode/reflection/beam:free")!;
    expect(beam.routes).toEqual([]);
    expect(beam.aaScore).toBeNull();
    expect(beam.plan).toBe("Command Code GOAT");
    expect(beam.multiplier).toBe(7);
  });

  test("unparseable config omits the catalog with the coverage", () => {
    const section = aaSectionFromSupabase(rawPayload(), "{ nope", "2026-10-26T18:05:00Z");
    expect(section?.catalog).toBeUndefined();
  });

  test("unparseable config omits coverage but keeps the section", () => {
    const section = aaSectionFromSupabase(rawPayload(), "{ nope", "2026-10-26T18:05:00Z");
    expect(section?.status).toBe("ok");
    expect(section?.coverage).toBeUndefined();
    expect(isAaSection(section)).toBe(true);
  });

  test("isAaSection accepts the absent form, rejects junk", () => {
    expect(isAaSection(aaAbsent("Supabase unreachable", "2026-10-26T18:05:00Z"))).toBe(true);
    expect(isAaSection(null)).toBe(false);
    expect(isAaSection({ status: "ok" })).toBe(false);
    expect(isAaSection({ status: "absent", reason: 42 })).toBe(false);
    expect(isAaSection({ generatedAt: "x", batch: "y", rankings: [] })).toBe(false);
  });

  test("summarizeAa: usable-up-to + snapshot vintage", () => {
    const summary = summarizeAa(FIXTURE_AA);
    const lunaKey = FIXTURE_AA.testRuns[0].testRunKey;
    expect(summary.usableUpTo.get(lunaKey)).toBe(30);
    expect(summary.snapshotVintage).toEqual({ min: "2026-09-28", max: "2026-09-28" });
  });
});

describe("explorer points (charts + perspectives)", () => {
  // A dominates B on score+cost+time; C is cheapest but slow — on the
  // cost frontier, off the time frontier, beaten by nobody on all
  // three axes at once (A costs more). The reference design's exact
  // semantics, in data form.
  function sectionWith(rows: { family: string; score: number; cost: number; time: number }[]): AaSection {
    const rankings: AaRanking[] = rows.map((r, i) => ({
      rowIndex: i,
      testRunKey: `k${i}`,
      modelVariantKey: `k${i}`,
      family: r.family,
      creator: "Test",
      effort: "max",
      benchmarkVersionKey: "aa-v4.3.2",
      rawScore: r.score,
      adjustedScore: r.score,
      costTask: r.cost,
      timeTaskSec: r.time,
      speedTps: 100,
      qualStatus: null,
      availability: "yes",
      eligible: true,
      intelEffPerUsd: null,
      usdPerUsableUnit: null,
      pointsPerDollar: null,
      pointsPerSecond: null,
      balancedValue: null,
      smartValue: null,
      speedValue: null,
      rankBalanced: null,
      rankSmart: null,
      rankSpeed: null,
      rankIntelligence: null,
      rankLowCost: null,
    }));
    return {
      status: "ok" as const,
      generatedAt: "2026-10-26T18:05:00Z",
      batch: "b",
      sources: [],
      rankings,
      thresholds: [],
      configs: [],
      testRuns: rows.map(
        (r, i): AaTestRun => ({
          testRunKey: `k${i}`,
          modelVariantKey: `k${i}`,
          snapshotDate: "2026-09-28",
          intelligence: r.score,
          costPerTaskUsd: r.cost,
          outputSpeedTps: 100,
          ttftSeconds: 5,
          firstAnswerSeconds: null,
          e2e500Seconds: null,
          timePerTaskSeconds: r.time,
        }),
      ),
    };
  }

  test("join carries TTFT from Test_Runs and derives per-dollar / per-second", () => {
    const pts = aaPoints(
      sectionWith([
        { family: "A", score: 50, cost: 1, time: 100 },
        { family: "B", score: 40, cost: 2, time: 200 },
        { family: "C", score: 40, cost: 0.5, time: 300 },
      ]),
    );
    const [a, b, c] = pts;
    expect(a.ttft).toBe(5);
    expect(a.scorePerDollar).toBe(50);
    expect(a.scorePerSecond).toBe(0.5);
    expect(b.beatenBy).toEqual(["A max"]); // dominated on all three axes
    expect(a.beatenBy).toEqual([]);
    expect(c.beatenBy).toEqual([]); // A is smarter+faster but costs more
    expect(a.frontierCost && a.frontierTime && a.frontierCostTime).toBe(true);
    expect(b.frontierCost || b.frontierTime || b.frontierCostTime).toBe(false);
    expect(c.frontierCost).toBe(true); // cheapest at its score
    expect(c.frontierTime).toBe(false); // A is faster AND smarter
    expect(c.frontierCostTime).toBe(true); // nothing is cheaper AND faster
  });

  test("a point without a measured time is on no frontier and beaten by nobody", () => {
    const s = sectionWith([{ family: "A", score: 50, cost: 1, time: 100 }]);
    s.rankings.push({
      ...s.rankings[0],
      rowIndex: 9,
      testRunKey: "k9",
      modelVariantKey: "k9",
      family: "Ghost",
      timeTaskSec: null,
    });
    s.testRuns.push({ ...s.testRuns[0], testRunKey: "k9", modelVariantKey: "k9", timePerTaskSeconds: null });
    const ghost = aaPoints(s).find((p) => p.family === "Ghost")!;
    expect(ghost.time).toBeNull();
    expect(ghost.frontierCost).toBe(true); // cost×score needs no time
    expect(ghost.frontierTime).toBe(false);
    expect(ghost.frontierCostTime).toBe(false);
    expect(ghost.beatenBy).toEqual([]);
  });
});

describe("fleet catalog (Model Explorer)", () => {
  const CATALOG_SNIPPET = `{
    "disabled_providers": ["claude-code"],
    "provider": {
      "openai": { "name": "OpenAI", "models": {
        "gpt-6-luna": { "name": "GPT-6 Luna", "limit": { "context": 1050000, "output": 128000 },
          "variants": { "max": {}, "low": {}, "high": {} } },
      } },
      "claude-code": { "name": "Claude Code (CLI proxy)", "models": {
        "claude-haiku-5-5": { "name": "Haiku 5.5 (Claude Code)" },
      } },
      "opencode-go.personal": { "instanceOf": "opencode-go" },
      "opencode-go.lab": { "name": "OpenCode Go — Lab", "models": {
        "deepseek-v4.1-flash": { "name": "DeepSeek V4.1 Flash" },
      } },
    },
    "experimental": { "model_routes": {
      "inner": { "targets": ["openai/gpt-6-luna#max"] },
      "outer": { "targets": ["opencode-route/inner", { "model": "claude-code/claude-haiku-5-5", "variants": {} }] },
    } },
  }`;

  test("entries carry seats (nested routes expanded), limits, ladder-ordered variants, disabled flag, plan facts", () => {
    const catalog = aaCatalogFromConfig(CATALOG_SNIPPET, {
      configs: [
        {
          modelName: "GPT-6 Luna",
          benchmark: "AA Intelligence Index",
          effort: "Reasoning",
          intelligence: 38,
          costPerTaskUsd: 0.07,
          speedTps: 145,
        },
      ],
      rankings: [],
    });
    expect(catalog).not.toBeNull();
    expect(catalog!.length).toBe(3); // the instanceOf alias contributes none
    const luna = catalog!.find((e) => e.id === "openai/gpt-6-luna")!;
    expect(luna.routes).toEqual(["inner", "outer"]);
    expect(luna.contextLimit).toBe(1050000);
    expect(luna.variants).toEqual(["low", "high", "max"]);
    expect(luna.disabled).toBe(false);
    expect(luna.plan).toBe("ChatGPT plan");
    expect(luna.aaScore).toBe(38);
    expect(luna.aaCostPerTask).toBe(0.07);
    expect(luna.goCapUsd).toBeNull(); // a cap is a Go-door fact, not a model fact
    const haiku = catalog!.find((e) => e.id === "claude-code/claude-haiku-5-5")!;
    expect(haiku.disabled).toBe(true);
    expect(haiku.routes).toEqual(["outer"]);
    expect(haiku.aaScore).toBeNull();
    const dsFlash = catalog!.find((e) => e.id === "opencode-go.lab/deepseek-v4.1-flash")!;
    expect(dsFlash.goCapUsd).toBe(80); // published Go cap (docs reading)
    expect(dsFlash.plan).toBe("OpenCode Go — Lab");
  });

  test("null on unparseable config", () => {
    expect(aaCatalogFromConfig("{ nope", { configs: [], rankings: [] })).toBeNull();
  });
});

describe("Coding Agent Index points (Coding agents perspective)", () => {
  const row = (over: Partial<AaCaiRow>): AaCaiRow => ({
    modelName: "GPT-6.1 Sol (xhigh)",
    family: "GPT-6.1 Sol",
    creator: "OpenAI",
    effort: "xhigh",
    version: "v1.5",
    score: 63,
    cost: 1.04,
    wallSec: 931.2,
    tokensPerTask: 3212528,
    harness: "Codex",
    deepSwe: null,
    terminalBench: null,
    sweAtlas: null,
    snapshotDate: "2026-10-17",
    sourceUrl: null,
    ...over,
  });

  test("caiFamily strips only the row's own effort suffix", () => {
    expect(caiFamily("GPT-6.1 Sol (xhigh)", "xhigh")).toBe("GPT-6.1 Sol");
    expect(caiFamily("Beam 1.0", "not stated")).toBe("Beam 1.0");
    expect(caiFamily("Model (Special Edition)", "low")).toBe("Model (Special Edition)");
    expect(caiFamily("No Suffix", null)).toBe("No Suffix");
  });

  test("points keep the CAI measures and compute frontiers inside the CAI population only", () => {
    const pts = aaCaiPoints([
      row({}),
      row({
        modelName: "Muse Wonder Full (max)",
        family: "Muse Wonder Full",
        creator: "Meta",
        effort: "max",
        score: 68,
        cost: null,
        wallSec: 1725.6,
        harness: "Muse Code",
        tokensPerTask: null,
      }),
      row({
        modelName: "DeepSeek V4 Flash 0731 (max)",
        family: "DeepSeek V4 Flash 0731",
        creator: "DeepSeek",
        effort: "max",
        score: 39,
        cost: 0.0851,
        wallSec: 1096.8,
        harness: "Codex",
      }),
    ]);
    expect(pts.length).toBe(3);
    const sol = pts[0];
    expect(sol.score).toBe(63);
    expect(sol.time).toBe(931.2); // agent wall time rides the time slot
    expect(sol.tps).toBeNull(); // CAI publishes no decode speed — never invented
    expect(sol.harness).toBe("Codex");
    expect(sol.benchmark).toBe("Coding Agent Index v1.5");
    // Sol is beaten on all three axes by nobody here (Wonder has no cost).
    expect(sol.frontierCost).toBe(true);
    expect(sol.beatenBy).toEqual([]);
    const wonder = pts[1];
    expect(wonder.frontierCost).toBe(false); // no published cost -> no cost frontier
    const flash = pts[2];
    expect(flash.frontierCostTime).toBe(true); // cheapest, and nothing is cheaper+faster
  });

  test("catalog prices carry their rate window (peak/off-peak stay separate bands)", () => {
    const prices: AaPrice[] = [
      {
        modelName: "openai/gpt-6-luna",
        inputPer1M: 1.4,
        outputPer1M: 8,
        cacheReadPer1M: 0.14,
        cacheWritePer1M: null,
        contextMinTokens: null,
        contextMaxTokens: null,
        windowKind: "peak",
      },
      {
        modelName: "openai/gpt-6-luna",
        inputPer1M: 0.7,
        outputPer1M: 4,
        cacheReadPer1M: 0.07,
        cacheWritePer1M: null,
        contextMinTokens: null,
        contextMaxTokens: null,
        windowKind: "offpeak",
      },
    ];
    const catalog = aaCatalogFromConfig(CONFIG_SNIPPET, { configs: [], rankings: [] }, prices);
    const luna = catalog!.find((e) => e.id === "openai/gpt-6-luna")!;
    expect(luna.rate?.windowed).toBe(true);
    expect(luna.rate?.bands.map((b) => b.window).sort()).toEqual(["offpeak", "peak"]);
    expect(luna.rate?.bands.find((b) => b.window === "offpeak")?.input).toBe(0.7);
  });
});

describe("aa-explorer manifest + render", () => {
  test("PAGES carries the aa-explorer page with unique routes", () => {
    const page = PAGES.find((p) => p.id === "aa-explorer");
    expect(page?.title).toBe("Cost Explorer");
    expect(page?.route).toBe("/aa");
    expect(page?.api).toBe("/api/aa.json");
    const routes = PAGES.map((p) => p.route);
    expect(new Set(routes).size).toBe(routes.length);
  });

  test("fixtures render: Cost Explorer shell — both tabs, charts, filters, perspectives, explorer data", () => {
    const html = renderAa(FIXTURE_AA, PROV);
    expect(html).toContain("Cost Explorer");
    expect(html).toContain("AA Explorer");
    expect(html).toContain("Model Explorer");
    expect(html).toContain('data-xtab="models"');
    expect(html).toContain('id="xf-q"');
    expect(html).toContain('data-xp="plan"');
    expect(html).toContain('data-xp="api"');
    expect(html).toContain('data-xp="pertask"');
    expect(html).toContain('id="xpl-c1"');
    expect(html).toContain('id="xpl-c2"');
    expect(html).toContain('id="xpl-c3"');
    expect(html).toContain('id="xpl-c4"'); // tasks completed vs duration
    expect(html).toContain('id="xpl-c5"'); // multiples of the baseline
    expect(html).toContain('id="xpl-c6"'); // scores-only bars
    expect(html).toContain('data-sizeby="xpl-c1"');
    expect(html).toContain('data-dl="xpl-c2"');
    expect(html).toContain('data-tmode="tot"');
    expect(html).toContain('id="xf-baseline"');
    expect(html).toContain('id="xpl-tray"');
    expect(html).toContain('id="xpl-mtrows"');
    expect(html).toContain("goCapUsd"); // published Go caps ride the catalog JSON
    expect(html).toContain('id="xpl-data"');
    expect(html).toContain("frontierCost"); // enriched points ride the embedded JSON
    expect(html).toContain("GPT-6 Sol");
    expect(html).toContain("1m 34s"); // Luna max's 93.76s TTFT, formatted
    expect(html).toContain("26.7s"); // Haiku high's TTFT
    expect(html).toContain("not measured"); // the fixture's unmeasured flash row
    expect(html).toContain("commandcode/reflection/beam:free");
    expect(html).toContain("claude-code/claude-haiku-5-5"); // catalog entry, disabled door
    expect(html).toContain('data-persp="cai"'); // Coding agents perspective
    expect(html).toContain("caiPoints"); // the CAI point set rides the embedded JSON
    expect(html).toContain("GPT-6.1 Sol"); // a CAI family, in the observations table
    expect(html).toContain("Coding Agent Index observations");
    expect(html).toContain('id="xpl-pf-score"'); // Pareto frontier tables
    expect(html).toContain('id="xpl-pf-price"');
    expect(html).toContain('id="xf-pfcut"');
    expect(html).toContain('id="xpl-recs"'); // recommendations engine
    expect(html).toContain('data-rec="speedPriority"');
    expect(html).toContain('data-recpreset="fast"');
    expect(html).toContain('id="xpl-bandgroups"'); // band comparisons
    expect(html).toContain('id="xf-bandscale"');
    expect(html).toContain('data-pj="inputTokens"'); // task-assumptions projection
    expect(html).toContain('data-pjpreset="repo"');
    expect(html).toContain("Proj. $/task");
    expect(html).toContain('id="xf-ineq-mode"'); // inequality filters with Pareto gaps
    expect(html).toContain('id="xf-ineq-conds"');
    expect(html).toContain('id="xf-setup-list"'); // saved setups
    expect(html).toContain('id="xf-dl-json"'); // joined-data download
    expect(html).toContain('id="xf-dl-csv"');
    expect(html).toContain("8152e746"); // batch provenance
    expect(html).toContain("/api/aa.json");
  });

  test("null and absent sections render honest banners, no tables", () => {
    const missing = renderAa(null, PROV);
    expect(missing).toContain("has not reached the dashboard yet");
    const absent = renderAa(aaAbsent("Supabase query failed (HTTP 500)", "2026-10-26T18:05:00Z"), PROV);
    expect(absent).toContain("Supabase query failed (HTTP 500)");
    expect(absent).not.toContain("Rankings");
  });

  test("fixtureSource().aa() returns the fixture section", async () => {
    const section = await fixtureSource().aa();
    expect(section?.status).toBe("ok");
  });
});

let proc: Subprocess<"ignore", "pipe", "inherit"> | null = null;
let port = 0;
const tmp = mkdtempSync(join(tmpdir(), "agg-aa-"));

beforeAll(async () => {
  port = 6100 + Math.floor(Math.random() * 700);
  const commentsFile = join(tmp, "comments.json");
  writeFileSync(commentsFile, JSON.stringify({ version: 1, comments: [] }));
  proc = spawn({
    cmd: ["bun", "run", "src/server.ts"],
    cwd: import.meta.dir + "/..",
    env: {
      ...Bun.env,
      PM_AGG_PORT: String(port),
      PM_AGG_SOURCE: "fixtures",
      PM_AGG_COMMENTS_FILE: commentsFile,
      PM_AGG_REGISTRY_FILE: join(tmp, "registry.json"),
    },
    stdout: "pipe",
    stderr: "inherit",
  });
  const deadline = Date.now() + 8000;
  let ready = false;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/api/health`);
      if (res.ok) {
        ready = true;
        break;
      }
    } catch {
      /* not up yet */
    }
    await Bun.sleep(120);
  }
  expect(ready).toBe(true);
});

afterAll(() => {
  proc?.kill();
});

describe("aa routes (HTTP, fixtures)", () => {
  test("/aa serves the explorer and /api/aa.json the section", async () => {
    const page = await (await fetch(`http://127.0.0.1:${port}/aa`)).text();
    expect(page).toContain("Cost Explorer");
    expect(page).toContain("Model Explorer");
    expect(page).toContain("GPT-6 Sol");
    const section = (await (await fetch(`http://127.0.0.1:${port}/api/aa.json`)).json()) as {
      status: string;
      rankings: { family: string }[];
      coverage?: { catalogNotPooledUnmeasured: string[] };
      catalog?: { id: string; disabled: boolean }[];
    };
    expect(section.status).toBe("ok");
    expect(section.rankings.some((r) => r.family === "GPT-6 Sol")).toBe(true);
    expect(section.coverage?.catalogNotPooledUnmeasured).toContain("commandcode/reflection/beam:free");
    expect(section.catalog?.some((e) => e.id === "commandcode/reflection/beam:free")).toBe(true);
    expect(section.catalog?.find((e) => e.id === "claude-code/claude-haiku-5-5")?.disabled).toBe(true);
  });

  test("/aa loads the same browser computation module imported by Bun tests", async () => {
    const page = await (await fetch(`http://127.0.0.1:${port}/aa`)).text();
    expect(page).toContain('<script type="module">');
    expect(page).toContain('from "/assets/aa-explorer-computations.js"');
    const response = await fetch(`http://127.0.0.1:${port}/assets/aa-explorer-computations.js`);
    expect(response.status).toBe(200);
    expect(response.headers.get("content-type")).toContain("text/javascript");
    const asset = await response.text();
    expect(asset).toContain("export function computeRecommendations");
    expect(asset).toContain("export function projectTask");
  });
});
