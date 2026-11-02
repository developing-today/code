import { describe, expect, test } from "bun:test";
import type { BoardItem, Provenance, RoadmapRow } from "../src/model.ts";
import { renderBoardBody, renderItem, renderProjects, renderRoadmap } from "../src/render.ts";
import {
  SERVICE_LINKS,
  gristDocUrl,
  itemServiceLinks,
  kaneoProjectUrl,
  kaneoTaskUrl,
  serviceHomeLinks,
} from "../src/service-links.ts";
import { FIXTURE_ROADMAP } from "../src/sources/fixtures.ts";

const PROV: Provenance = {
  board: "x", roadmap: "x", items: "x", comments: "x", quota: "x", usage: "x",
  attention: "x", orchestration: "x", dns: "x", registry: "x", aa: "x",
};

describe("service-link registry (#1136)", () => {
  test("homes are browser forms — no loopback, no raw IPs", () => {
    for (const def of Object.values(SERVICE_LINKS)) {
      expect(def.home).not.toContain("127.0.0.1");
      expect(def.home).not.toContain("localhost");
    }
    expect(SERVICE_LINKS.kaneo.home).toBe("https://kaneo.security.cab");
    expect(SERVICE_LINKS.pad.home).toBe("http://amd-6.raptor-pumpkinseed.ts.net:7777");
    expect(SERVICE_LINKS.grist.home).toBe("http://amd-6.raptor-pumpkinseed.ts.net:8484");
  });

  test("kaneo deep links use the verified dashboard route", () => {
    expect(kaneoTaskUrl("ws1", "proj1", "task1")).toBe(
      "https://kaneo.security.cab/dashboard/workspace/ws1/project/proj1/task/task1",
    );
    expect(kaneoProjectUrl("ws1", "proj1")).toBe(
      "https://kaneo.security.cab/dashboard/workspace/ws1/project/proj1",
    );
  });

  test("grist doc link targets the roadmap document on the tailnet host", () => {
    expect(gristDocUrl("czYHYKgG9SHcm9CPwiKEq1")).toBe(
      "http://amd-6.raptor-pumpkinseed.ts.net:8484/doc/czYHYKgG9SHcm9CPwiKEq1",
    );
  });

  test("itemServiceLinks reads the URLs the sources set", () => {
    const item: BoardItem = {
      id: "kaneo:code/t1", title: "t", repo: "developing-today/code", source: "kaneo",
      status: "in-progress", kaneoUrl: kaneoTaskUrl("w", "p", "t1"), links: [],
    };
    expect(itemServiceLinks(item)).toEqual([
      { service: "kaneo", label: "Kaneo card", url: kaneoTaskUrl("w", "p", "t1") },
    ]);
    const row: RoadmapRow = {
      id: 1, subject: "grist:doc/Roadmap/1", title: "r", area: "a", status: "s",
      horizon: "h", source: "", notes: "", links: [], gristUrl: gristDocUrl("doc"),
    };
    expect(itemServiceLinks(row).map((l) => l.service)).toEqual(["grist"]);
    expect(serviceHomeLinks().map((l) => l.service)).toEqual(["kaneo", "grist", "pad", "ready-for-agent"]);
  });
});

describe("service links render (#1136)", () => {
  test("board cards link out to the Kaneo card", () => {
    const url = kaneoTaskUrl("w", "p", "t1");
    const item: BoardItem = {
      id: "kaneo:code/t1", title: "Card title", repo: "developing-today/code", source: "kaneo",
      status: "in-progress", kaneoUrl: url, links: [],
    };
    const html = renderBoardBody([item], new Map());
    expect(html).toContain(`href="${url}"`);
    expect(html).toContain("Kaneo ↗");
  });

  test("roadmap cards link out to the Grist document", () => {
    const html = renderRoadmap(FIXTURE_ROADMAP, new Map(), PROV);
    expect(html).toContain(`href="${gristDocUrl("czYHYKgG9SHcm9CPwiKEq1")}"`);
    expect(html).toContain("Grist ↗");
  });

  test("roadmap grain links preserve the active view query (#1276, Codex P2 on PR #4013)", () => {
    const q = "view=vw-1&vf.repo=developing-today%2Fcode";
    const html = renderRoadmap(FIXTURE_ROADMAP, new Map(), PROV, "quarter", undefined, undefined, undefined, undefined, undefined, q);
    expect(html).toContain(`href="/roadmap?h=month&${q}"`);
    expect(html).toContain(`href="/roadmap?h=quarter&${q}"`);
    const plain = renderRoadmap(FIXTURE_ROADMAP, new Map(), PROV);
    expect(plain).toContain('href="/roadmap?h=month"');
    expect(plain).not.toContain("vf.repo");
  });

  test("item pages carry a Live services line", () => {
    const url = kaneoTaskUrl("w", "p", "t1");
    const html = renderItem("Card title", "kaneo:code/t1", [], [], new Map(), [], PROV, {
      dashComments: [],
      formRef: "kaneo:code/t1",
      formOpts: { identities: [] },
      sessions: [],
      serviceLinks: [{ service: "kaneo", label: "Kaneo card", url }],
    });
    expect(html).toContain("Live services:");
    expect(html).toContain(`href="${url}"`);
  });

  test("projects page carries the service strip", () => {
    const html = renderProjects([], PROV);
    expect(html).toContain("Live services:");
    expect(html).toContain('href="https://kaneo.security.cab"');
    expect(html).toContain('href="http://amd-6.raptor-pumpkinseed.ts.net:7777"');
  });
});
