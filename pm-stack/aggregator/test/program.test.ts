import { describe, expect, test } from "bun:test";
import type { GhItem, OrchestrationSection, ProjectPoolItem, Provenance } from "../src/model.ts";
import type { CoverageSnapshotFile } from "../src/coverage.ts";
import { PAGES, pageByRoute } from "../src/pages.ts";
import {
  PROJECT_CONFIGS,
  allProjectRefs,
  buildProjectDetail,
  buildProjectSummaries,
  projectConfigById,
  unsetPoolCount,
  validateProjectConfigs,
  type ProjectConfig,
} from "../src/project-config.ts";
import { buildProgramData, buildProgramLive, PROGRAM, programRefs, TRACKED_REFS } from "../src/program.ts";
import { projectsBodyHtml, renderProgram, renderProgramProject } from "../src/render.ts";
import { FIXTURE_ROADMAP, fixtureSource } from "../src/sources/fixtures.ts";

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
  aa: "x",
};

describe("stale issue coverage panel", () => {
  test("shows per-repo count, collection time, and stale/error/unavailable states honestly", () => {
    const coverage: CoverageSnapshotFile = {
      baseline: null,
      current: {
        generatedAt: "2026-11-01T12:00:00.000Z",
        repos: [
          {
            repo: "org/stale",
            openIssues: 10,
            gateNow: 0,
            gateNext: 0,
            poolItems: null,
            poolUnsetStatus: null,
            poolUnsetArea: null,
            staleOpenIssues: 4,
            staleIssuesCollectedAt: "2026-10-01T12:00:00.000Z",
            staleIssuesError: null,
          },
          {
            repo: "org/error",
            openIssues: null,
            gateNow: null,
            gateNext: null,
            poolItems: null,
            poolUnsetStatus: null,
            poolUnsetArea: null,
            staleOpenIssues: null,
            staleIssuesCollectedAt: "2026-11-01T11:59:00.000Z",
            staleIssuesError: "search 503 <retry>",
          },
          {
            repo: "org/missing",
            openIssues: null,
            gateNow: null,
            gateNext: null,
            poolItems: null,
            poolUnsetStatus: null,
            poolUnsetArea: null,
            staleOpenIssues: null,
            staleIssuesCollectedAt: null,
            staleIssuesError: null,
          },
        ],
      },
    };
    const html = projectsBodyHtml([], { coverage });
    expect(html).toContain("Open issues untouched &gt;90 days");
    expect(html).toContain(">4</td><td><time datetime=\"2026-10-01T12:00:00.000Z\">2026-10-01T12:00:00.000Z</time></td><td>STALE — collection is over 7 days old</td>");
    expect(html).toContain("Collection error: search 503 &lt;retry&gt;");
    expect(html).toContain("Unavailable — collection failed or not collected");
    expect(html).toContain(">—</td>");
  });
});

function poolItem(partial: Partial<ProjectPoolItem>): ProjectPoolItem {
  return {
    id: "PVTI_x",
    url: "https://github.com/orgs/developing-today/projects/4?pane=issue&itemId=1",
    projectUrl: "https://github.com/orgs/developing-today/projects/4",
    ref: "developing-today/code#1",
    repo: "developing-today/code",
    number: 1,
    issueUrl: "https://github.com/developing-today/code/issues/1",
    title: "item",
    state: "open",
    contentKind: "issue",
    status: null,
    area: null,
    horizon: null,
    kind: null,
    ...partial,
  };
}

describe("page manifest (#1046)", () => {
  test("PAGES contains program with the right route + api twin", () => {
    const page = PAGES.find((p) => p.id === "program");
    expect(page).toBeDefined();
    expect(page?.title).toBe("Program status");
    expect(page?.route).toBe("/program");
    expect(page?.api).toBe("/api/program.json");
    expect(pageByRoute("/program")?.id).toBe("program");
  });
});

describe("program data builder", () => {
  test("handles empty pool, absent items, and null orchestration", () => {
    const live = buildProgramLive(new Map(), [], null);
    expect(live.pool.total).toBe(0);
    expect(live.pool.byStatus).toEqual([]);
    expect(live.pool.byArea).toEqual([]);
    expect(live.orchestration).toBeNull();
    // Every tracked ref is present and unresolved — never dropped,
    // never given an invented state.
    expect(live.tracked.map((t) => t.ref)).toEqual(TRACKED_REFS);
    expect(live.tracked.every((t) => t.item === null)).toBe(true);
  });

  test("derives pool counts by status and area, and orchestration freshness", () => {
    const pool = [
      poolItem({ status: "Done", area: "pm-dashboard" }),
      poolItem({ status: "Done", area: "fleet-ops" }),
      poolItem({ status: "Todo", area: "pm-dashboard" }),
      poolItem({ status: null, area: null }),
    ];
    const section = {
      generatedAt: "2026-10-26T17:00:00Z",
      runId: "orch-2026-10-26T17:00:00Z",
    } as OrchestrationSection;
    const live = buildProgramLive(new Map(), pool, section);
    expect(live.pool.total).toBe(4);
    expect(live.pool.byStatus).toContainEqual({ name: "Done", count: 2 });
    expect(live.pool.byStatus).toContainEqual({ name: "(unset)", count: 1 });
    expect(live.pool.byArea).toContainEqual({ name: "pm-dashboard", count: 2 });
    expect(live.orchestration).toEqual({
      generatedAt: "2026-10-26T17:00:00Z",
      runId: "orch-2026-10-26T17:00:00Z",
    });
  });

  test("programRefs covers the tracked refs and every entry ref", () => {
    const refs = programRefs();
    for (const r of TRACKED_REFS) expect(refs).toContain(r);
    expect(refs).toContain("developing-today/code#911");
    expect(new Set(refs).size).toBe(refs.length); // de-duplicated
  });
});

describe("renderProgram (projects index, #1121)", () => {
  test("renders the index: every configured project card, live blocks, unset gap", async () => {
    const src = fixtureSource();
    const items = await src.items(programRefs());
    const pool = await src.projects();
    const rows = await src.roadmap();
    const data = {
      ...buildProgramData(items, pool, await src.orchestration()),
      projects: buildProjectSummaries(PROJECT_CONFIGS, pool, rows),
    };
    const html = renderProgram(data, items, PROV);
    expect(html).toContain("Program status — projects");
    // One card + route per configured project.
    for (const c of PROJECT_CONFIGS) {
      expect(html).toContain(c.title.replace(/&/g, "&amp;"));
      expect(html).toContain(`href="/program/${c.id}"`);
    }
    // The pm-dashboard tracked table rides on the index.
    expect(html).toContain("developing-today/code#910");
    expect(html).toContain("unresolved");
    // Fixtures carry no pool and no orchestration: both say so.
    expect(html).toContain("No Project pool in the current feed");
    expect(html).toContain("no orchestration section in the current feed yet");
    // The design record + Project links are present.
    expect(html).toContain("2026-10-24T11-05-38Z_design_project-management-stack");
    expect(html).toContain("https://github.com/orgs/developing-today/projects/4");
    // The full pm doc is one click away, not gone.
    expect(html).toContain('href="/program/pm-dashboard"');
  });

  test("index shows pool counts, freshness, and the unset-Area gap when data exists", () => {
    const item: GhItem = {
      ref: "developing-today/code#910",
      kind: "issue",
      url: "https://github.com/developing-today/code/issues/910",
      title: "Org Project layer",
      state: "closed",
      labels: [],
      author: "dezren39",
      updatedAt: "2026-10-26T17:00:00Z",
    };
    const items = new Map([[item.ref, item]]);
    const pool = [poolItem({ status: "Done", area: "pm-dashboard" }), poolItem({ status: "Todo", area: null })];
    const section = {
      generatedAt: "2026-10-26T17:00:00Z",
      runId: "orch-2026-10-26T17:00:00Z",
    } as OrchestrationSection;
    const data = {
      ...buildProgramData(items, pool, section),
      projects: buildProjectSummaries(PROJECT_CONFIGS, pool, FIXTURE_ROADMAP),
    };
    const html = renderProgram(data, items, PROV);
    expect(html).toContain("Org Project layer");
    expect(html).toContain("2 items");
    expect(html).toContain("Done: 1");
    expect(html).toContain("orch-2026-10-26T17:00:00Z");
    expect(html).toContain("carry no Area");
    expect(PROGRAM.done.length).toBeGreaterThan(0);
  });
});

// --- Project registry (#1120) -------------------------------------------------

describe("project registry", () => {
  test("the shipped registry is valid and has the six v1 projects", () => {
    expect(validateProjectConfigs(PROJECT_CONFIGS)).toEqual([]);
    expect(PROJECT_CONFIGS.map((c) => c.id)).toEqual([
      "pm-dashboard",
      "lab-cluster",
      "id",
      "repo-fleet",
      "mcpx",
      "cloud-program",
    ]);
    // pm-dashboard migrated with no content loss: editorial IS the doc.
    const pm = projectConfigById("pm-dashboard");
    expect(pm?.editorial?.done).toBe(PROGRAM.done);
    expect(pm?.editorial?.next).toBe(PROGRAM.next);
    expect(pm?.architecture).toBe(PROGRAM.architecture);
    expect(pm?.trackedRefs).toBe(TRACKED_REFS);
    expect(projectConfigById("nope")).toBeUndefined();
  });

  test("researched editorial slices are in the shipped registry (#1124/#1125/#1126/#1128)", () => {
    const lab = projectConfigById("lab-cluster")!;
    expect(lab.statusReport?.label).toContain("2026-10-27");
    expect(lab.statusReport?.url).toContain("doc/2026-10-27T03-14-08Z_status_basement-cluster");
    const labGaps = (lab.editorial?.gaps ?? []).map((e) => `${e.title} ${e.detail}`).join("\n");
    expect(labGaps).toContain("CPCOE-29381");
    expect(labGaps).toContain("2026-10-21");
    expect(lab.editorial?.gaps.some((e) => (e.refs ?? []).includes("developing-today/code#1091"))).toBe(true);

    // #1124 residuals: the pre-seed dezren39/nix#501-#508 predecessor set is
    // tracked and explained, and the Grist-vs-Project status disagreement
    // for #1080/#1081 is surfaced as an honest gap naming both readings.
    for (const n of [501, 502, 503, 504, 505, 506, 507, 508]) {
      expect(lab.trackedRefs).toContain(`dezren39/nix#${n}`);
    }
    expect(labGaps).toContain("dezren39/nix (#501-#508)");
    expect(labGaps).toContain("Hardware inventory + firmware/BMC audit");
    expect(labGaps).toContain("In progress / Q4 2026");
    expect(labGaps).toContain("Todo / planned");
    const disagreement = (lab.editorial?.gaps ?? []).find((e) => e.title.includes("Status sources disagree"));
    expect(disagreement?.refs).toContain("developing-today/code#1080");
    expect(disagreement?.refs).toContain("developing-today/code#1081");
    expect(disagreement?.refs).toContain("dezren39/nix#501");
    expect(disagreement?.refs).toContain("dezren39/nix#502");

    const id = projectConfigById("id")!;
    for (const n of [1076, 1077, 1078, 1235, 1239, 1240]) {
      expect(id.trackedRefs).toContain(`developing-today/code#${n}`);
    }
    const idLinks = (id.dashboardLinks ?? []).map((l) => l.route);
    for (const doc of ["ARCHITECTURE.md", "README.md", "TODO.md"]) {
      expect(idLinks.some((r) => r.endsWith(`pkgs/id/${doc}`))).toBe(true);
    }
    expect((id.editorial?.done ?? []).map((e) => e.title).join("\n")).toContain("validation in progress");
    // #1125 coverage-gap disclosure: the id page must state that its pool
    // (Area=id-train) is a subset of the open app/id issues, with a dated
    // count — never implying the pool is the whole project.
    const idGaps = (id.editorial?.gaps ?? []).map((e) => `${e.title} ${e.detail}`).join("\n");
    expect(idGaps).toContain("id-train");
    expect(idGaps).toContain("app/id");
    expect(idGaps).toMatch(/\d+ open app\/id issues on \d{4}-\d{2}-\d{2}/);
    expect(id.editorial?.gaps.some((e) => (e.refs ?? []).includes("developing-today/code#1125"))).toBe(true);
    expect((id.editorial?.done ?? []).map((e) => e.title)).not.toContain("Phases 9–11 delivered");

    const fleet = projectConfigById("repo-fleet")!;
    const fleetUpcoming = [...(fleet.editorial?.next ?? []), ...(fleet.editorial?.longTerm ?? [])];
    const fleetUpcomingText = fleetUpcoming.map((e) => `${e.title} ${e.detail}`).join("\n");
    for (const n of [1233, 1130, 1131, 1234, 1185, 1187, 1190, 310, 311]) {
      expect(fleet.trackedRefs).toContain(`developing-today/code#${n}`);
    }
    expect(fleetUpcomingText).toContain("Queue fleet reliability");
    expect(fleetUpcomingText).toContain("OpenChamber session dispatch + babysitter");
    expect(fleetUpcomingText).toContain("stale-PR triage");
    // Closed #257/#292 are recorded as done, never presented as upcoming.
    expect(fleetUpcoming.flatMap((e) => e.refs ?? [])).not.toContain("developing-today/code#257");
    expect(fleetUpcoming.flatMap((e) => e.refs ?? [])).not.toContain("developing-today/code#292");
    const fleetDone = (fleet.editorial?.done ?? []).map((e) => `${e.title} ${e.detail}`).join("\n");
    expect(fleetDone).toContain("closed in GitHub");
    const fleetGaps = (fleet.editorial?.gaps ?? []).map((e) => `${e.title} ${e.detail}`).join("\n");
    expect(fleetGaps).toContain("2026-10-29");
    expect(fleetGaps).toContain("1,119 open issues");

    const cloud = projectConfigById("cloud-program")!;
    expect(cloud.roadmapArea).toBe("Cloud program");
    // #1128 area decision: links+refs for the pool, Grist area kept.
    expect(cloud.poolArea).toBeNull();
    const cloudNext = (cloud.editorial?.next ?? []).map((e) => `${e.title} ${e.detail}`).join("\n");
    expect(cloudNext).toContain("OCI signup resolution");
    expect(cloudNext).toContain("CPCOE-29381");
    expect(cloudNext).toContain("Free-services pass");
    // The 2026-10-26 SaaS review list must stay named in the entry.
    for (const svc of [
      "boats.cloud",
      "quic.cloud",
      "Backblaze B2",
      "OVHcloud",
      "Railway",
      "Render",
      "DigitalOcean",
      "Hetzner",
      "Heroku",
      "GCP",
      "AWS",
    ]) {
      expect(cloudNext).toContain(svc);
    }
    // boats.cloud caveat (CodeRabbit nitpick on PR #2406): it stays unverified and untouched.
    expect(cloudNext).toContain("unverified");
    expect(cloudNext).toContain("untouched");
    const cloudLongTerm = (cloud.editorial?.longTerm ?? []).map((e) => `${e.title} ${e.detail}`).join("\n");
    expect(cloudLongTerm).toContain("Egress offload");
    expect(cloudLongTerm).toContain("zero-egress");
    expect(cloudLongTerm).toContain("Always-free maximization");
  });

  test("repo-fleet carries four per-repo cards and a sweep-health section (#1126)", () => {
    const fleet = projectConfigById("repo-fleet")!;
    const cards = fleet.repoCards ?? [];
    expect(cards.map((c) => c.repo)).toEqual([
      "developing-today/code",
      "developing-today/opencode",
      "developing-today/openchamber",
      "dezren39/nix",
    ]);
    for (const card of cards) {
      expect(card.role.length).toBeGreaterThan(0);
      // Dated snapshot counts, taken 2026-10-29 — never live claims.
      expect(card.openIssuesAsOf).toBe("2026-10-29");
      expect(card.openIssues).toBeGreaterThan(0);
      expect(card.trackedRefs.length).toBeGreaterThan(0);
    }
    expect(cards.reduce((n, c) => n + c.openIssues, 0)).toBe(1119);
    // Sweep health rests on published fleet data only (orchestration
    // section + tracked issues), and says so.
    expect((fleet.sweepHealth ?? []).length).toBeGreaterThan(0);
    const sweepText = (fleet.sweepHealth ?? []).map((e) => `${e.title} ${e.detail}`).join("\n");
    expect(sweepText).toContain("orchestration");
    // The scope note is refreshed, dated, and claims no completeness.
    const gaps = (fleet.editorial?.gaps ?? []).map((e) => `${e.title} ${e.detail}`).join("\n");
    expect(gaps).toContain("2026-10-29");
    expect(gaps).toContain("does not claim pool completeness");
    // Card + sweep-health refs resolve through the shared ref set.
    const refs = allProjectRefs();
    expect(refs).toContain("developing-today/opencode#365");
    expect(refs).toContain("developing-today/openchamber#214");
    expect(refs).toContain("dezren39/nix#512");
  });

  test("validation catches duplicates and malformed entries", () => {
    const base: ProjectConfig = {
      id: "x-proj",
      title: "X",
      intro: "x",
      poolArea: "area-x",
      roadmapArea: "Area X",
      trackedRefs: ["developing-today/code#1"],
    };
    const dup: ProjectConfig = { ...base, title: "X2" };
    const errors = validateProjectConfigs([base, dup]);
    expect(errors.some((e) => e.includes("duplicate project id"))).toBe(true);
    expect(errors.some((e) => e.includes("poolArea"))).toBe(true);
    expect(errors.some((e) => e.includes("roadmapArea"))).toBe(true);
    const bad = validateProjectConfigs([
      { ...base, id: "Not A Slug", poolArea: null, roadmapArea: null, trackedRefs: ["nope"] },
    ]);
    expect(bad.some((e) => e.includes("not a slug"))).toBe(true);
    expect(bad.some((e) => e.includes("malformed ref"))).toBe(true);
    const badLinks = validateProjectConfigs([
      {
        ...base,
        statusReport: { label: " ", url: "http://example.com/report" },
        dashboardLinks: [{ label: "", route: "example.com/doc" }],
      },
    ]);
    expect(badLinks.some((e) => e.includes("statusReport.label is empty"))).toBe(true);
    expect(badLinks.some((e) => e.includes("statusReport.url is not an https URL"))).toBe(true);
    expect(badLinks.some((e) => e.includes("dashboardLinks label is empty"))).toBe(true);
    expect(badLinks.some((e) => e.includes("dashboardLinks route"))).toBe(true);
  });

  test("summaries derive pool/roadmap counts per project; unset is counted once", () => {
    const pool = [
      poolItem({ status: "Done", area: "pm-dashboard" }),
      poolItem({ status: "Todo", area: "pm-dashboard" }),
      poolItem({ status: "Todo", area: "id-train" }),
      poolItem({ status: "Todo", area: "mcpx" }),
      poolItem({ status: "Todo", area: "mcpx" }),
      poolItem({ status: null, area: null }),
    ];
    // The live Grist table gained mcpx rows 24-27 in the #1127 taxonomy pass;
    // the in-repo seed predates them, so the wired-area rows are added here.
    const roadmap = [
      ...FIXTURE_ROADMAP,
      {
        id: 24,
        subject: "grist:fixture/Roadmap/24",
        title: "mcpx — single source: consumption decision + backlog re-home",
        area: "mcpx",
        status: "In progress",
        horizon: "Now",
        source: "https://github.com/developing-today/code/issues/308",
        notes: "",
        links: [],
      },
      {
        id: 25,
        subject: "grist:fixture/Roadmap/25",
        title: "mcpx — supervisor, session resume, ExecIdentity",
        area: "mcpx",
        status: "Planned",
        horizon: "Next",
        source: "",
        notes: "",
        links: [],
      },
    ];
    const summaries = buildProjectSummaries(PROJECT_CONFIGS, pool, roadmap);
    const byId = new Map(summaries.map((s) => [s.id, s]));
    expect(byId.get("pm-dashboard")?.poolTotal).toBe(2);
    expect(byId.get("pm-dashboard")?.poolByStatus).toContainEqual({ name: "Done", count: 1 });
    expect(byId.get("pm-dashboard")?.roadmapRows).toBe(5); // fixture seed "PM stack"
    expect(byId.get("id")?.poolTotal).toBe(1);
    expect(byId.get("id")?.roadmapRows).toBe(7); // fixture seed area "id"
    expect(byId.get("repo-fleet")?.roadmapRows).toBe(4);
    expect(byId.get("lab-cluster")?.roadmapRows).toBe(6);
    // mcpx is wired to its own Area (#1127): its fixture pool items and
    // roadmap rows count, where a null Area would claim none.
    expect(byId.get("mcpx")?.poolTotal).toBe(2);
    expect(byId.get("mcpx")?.roadmapRows).toBe(2);
    // cloud-program has no pool Area, but its Grist area exists (#1128):
    // the fixture seed's one "Cloud program" row counts.
    expect(byId.get("cloud-program")?.poolTotal).toBe(0);
    expect(byId.get("cloud-program")?.roadmapRows).toBe(1);
    expect(unsetPoolCount(pool)).toBe(1);
  });

  test("allProjectRefs covers every project's tracked + editorial refs, de-duplicated", () => {
    const refs = allProjectRefs();
    for (const r of TRACKED_REFS) expect(refs).toContain(r);
    expect(refs).toContain("developing-today/code#1080"); // lab-cluster epic
    expect(refs).toContain("developing-today/code#911"); // repo-fleet + pm next
    expect(refs).toContain("dezren39/nix#254"); // mcpx editorial
    expect(new Set(refs).size).toBe(refs.length);
  });
});

// --- Per-project pages (#1121) --------------------------------------------------

describe("renderProgramProject", () => {
  test("every configured project renders from fixtures, nulls stated honestly", async () => {
    const src = fixtureSource();
    const items = await src.items(allProjectRefs());
    const pool = await src.projects();
    const rows = await src.roadmap();
    for (const config of PROJECT_CONFIGS) {
      const html = renderProgramProject(buildProjectDetail(config, items, pool, rows), items, PROV);
      expect(html).toContain(config.title.replace(/&/g, "&amp;"));
      expect(html).toContain("Live status — tracked issues");
      expect(html).toContain("Project pool — live");
      expect(html).toContain("Roadmap — live");
    }
    // mcpx is wired to its own Area (#1127): with no mcpx pool items or
    // roadmap rows in the fixture feed it states the empty feed honestly,
    // not the old null-area gap.
    const mcpx = projectConfigById("mcpx")!;
    const mcpxHtml = renderProgramProject(buildProjectDetail(mcpx, items, pool, rows), items, PROV);
    expect(mcpxHtml).not.toContain("No dedicated Project Area yet");
    expect(mcpxHtml).not.toContain("No Grist roadmap area yet");
    expect(mcpxHtml).toContain("No pool items with Area");
    expect(mcpxHtml).toContain("No roadmap rows with area");
    expect(mcpxHtml).toContain("planning/roadmaps/mcpx.md");
    expect(mcpxHtml).toContain("pkgs/mcpx/TODO.md");
    const cloud = projectConfigById("cloud-program")!;
    const cloudHtml = renderProgramProject(buildProjectDetail(cloud, items, pool, rows), items, PROV);
    expect(cloudHtml).toContain("No home doc configured");
    expect(cloudHtml).toContain("/quota");
    // Its Grist area exists (#1128): the seed's OCI row renders.
    expect(cloudHtml).toContain("OCI Always Free — run the technical maximum");
    expect(cloudHtml).not.toContain("No Grist roadmap area yet");

    const lab = projectConfigById("lab-cluster")!;
    const labHtml = renderProgramProject(buildProjectDetail(lab, items, pool, rows), items, PROV);
    expect(labHtml).toContain("Status report:");
    expect(labHtml).toContain("doc/2026-10-27T03-14-08Z_status_basement-cluster");
    // #1123: projects without a statusReport say so explicitly instead of
    // silently omitting the layer; lab-cluster (the one project with a
    // report) must not carry the note.
    expect(labHtml).not.toContain("No status report yet");
    expect(mcpxHtml).toContain("No status report yet for this project.");
    expect(cloudHtml).toContain("No status report yet for this project.");
    expect(labHtml).toContain("CPCOE-29381");

    const id = projectConfigById("id")!;
    const idHtml = renderProgramProject(buildProjectDetail(id, items, pool, rows), items, PROV);
    expect(idHtml).toContain("pkgs/id/ARCHITECTURE.md");
    expect(idHtml).toContain("pkgs/id/TODO.md");
    expect(idHtml).toContain("idle planning");
    expect(idHtml).toContain("Phases 9–11 delivered; validation in progress");

    const fleet = projectConfigById("repo-fleet")!;
    const fleetHtml = renderProgramProject(buildProjectDetail(fleet, items, pool, rows), items, PROV);
    expect(fleetHtml).toContain("Queue fleet reliability");
    expect(fleetHtml).toContain("1,119 open issues");
    // Per-repo cards + sweep health render (#1126).
    expect(fleetHtml).toContain("Per-repo cards");
    for (const repo of ["developing-today/code", "developing-today/opencode", "developing-today/openchamber", "dezren39/nix"]) {
      expect(fleetHtml).toContain(repo);
    }
    expect(fleetHtml).toContain("Open issues (GitHub search, 2026-10-29)");
    expect(fleetHtml).toContain("Sweep health — published fleet state");
    expect(fleetHtml).toContain("No new data pipeline feeds this section");
  });

  test("detail builder splits pool and roadmap by repo for the fleet cards (#1126)", () => {
    const pool = [
      poolItem({ area: "fleet-ops", repo: "developing-today/code", ref: "developing-today/code#1", title: "code item" }),
      poolItem({ area: "fleet-ops", repo: "dezren39/nix", ref: "dezren39/nix#2", number: 2, title: "nix item" }),
      poolItem({ area: "pm-dashboard", repo: "developing-today/code", ref: "developing-today/code#3", number: 3, title: "other area" }),
    ];
    const fleet = projectConfigById("repo-fleet")!;
    const detail = buildProjectDetail(fleet, new Map(), pool, FIXTURE_ROADMAP);
    expect(detail.repoCards.map((c) => c.card.repo)).toEqual([
      "developing-today/code",
      "developing-today/opencode",
      "developing-today/openchamber",
      "dezren39/nix",
    ]);
    const byRepo = new Map(detail.repoCards.map((c) => [c.card.repo, c]));
    expect(byRepo.get("developing-today/code")?.poolItems.map((p) => p.title)).toEqual(["code item"]);
    expect(byRepo.get("dezren39/nix")?.poolItems.map((p) => p.title)).toEqual(["nix item"]);
    expect(byRepo.get("developing-today/opencode")?.poolItems).toEqual([]);
    // Fixture roadmap rows name their repo in the title / links.
    expect(byRepo.get("developing-today/opencode")?.roadmapRows.length).toBeGreaterThan(0);
    expect(byRepo.get("developing-today/openchamber")?.roadmapRows.length).toBeGreaterThan(0);
  });

  test("pm-dashboard page retains the full #1046 document", async () => {
    const src = fixtureSource();
    const items = await src.items(allProjectRefs());
    const pm = projectConfigById("pm-dashboard")!;
    const html = renderProgramProject(
      buildProjectDetail(pm, items, await src.projects(), await src.roadmap()),
      items,
      PROV,
    );
    for (const heading of [
      "What this is — architecture",
      "What's built",
      "What's next",
      "Long-term roadmap",
      "Known gaps",
    ]) {
      expect(html).toContain(heading);
    }
    expect(html).toContain("Wire the work-picking jobs to the orchestrator");
    expect(html).toContain("Cozystack");
    expect(html).toContain("2026-10-24T11-05-38Z_design_project-management-stack");
  });

  test("detail builder filters pool by Area and rows by roadmap area", () => {
    const pool = [
      poolItem({ status: "Done", area: "id-train", title: "b item" }),
      poolItem({ status: "Todo", area: "id-train", title: "a item" }),
      poolItem({ status: "Todo", area: "pm-dashboard", title: "other" }),
      poolItem({ status: null, area: null, title: "unset" }),
    ];
    const id = projectConfigById("id")!;
    const detail = buildProjectDetail(id, new Map(), pool, FIXTURE_ROADMAP);
    expect(detail.poolItems.map((p) => p.title)).toEqual(["b item", "a item"]); // status-sorted
    expect(detail.poolByStatus).toContainEqual({ name: "Done", count: 1 });
    expect(detail.roadmapRows.length).toBe(7);
    expect(detail.roadmapRows.every((r) => r.area === "id")).toBe(true);
    expect(detail.unsetPoolCount).toBe(1);
    expect(detail.tracked.every((t) => t.item === null)).toBe(true); // unresolved, not invented
  });
});
