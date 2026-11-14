import { describe, expect, test } from "bun:test";
import { buildAttention, deriveToday } from "../src/derive.ts";
import { renderBoardBody, renderTodayBody } from "../src/render.ts";
import type { BoardItem, ProjectPoolItem, QuotaView } from "../src/model.ts";
import { summarizeWipLanes, WIP_CAPS, type WipCapConfig } from "../src/wip-limits.ts";

const PROJECT = "https://github.com/orgs/developing-today/projects/4";
const EPIC = "developing-today/code#1184";

function boardItem(index: number, status = "in-progress"): BoardItem {
  const ref = `developing-today/code#${2000 + index}`;
  return {
    id: `kaneo:code/${index}`,
    title: `Work ${index}`,
    repo: "developing-today/code",
    source: "kaneo",
    status,
    links: [{ ref, relation: "tracks" }],
  };
}

function poolItem(index: number): ProjectPoolItem {
  const ref = `developing-today/code#${2000 + index}`;
  return {
    id: `project-item-${index}`,
    url: `${PROJECT}/items/${index}`,
    projectUrl: PROJECT,
    ref,
    repo: "developing-today/code",
    number: 2000 + index,
    issueUrl: `https://github.com/${ref.replace("#", "/issues/")}`,
    title: `Work ${index}`,
    state: "open",
    contentKind: "issue",
    status: "In progress",
    area: "PM stack",
    horizon: "Now",
    kind: "Task",
    epicRef: EPIC,
  };
}

const emptyQuota = {
  pools: [],
  burn: [],
  choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] },
} as QuotaView;

describe("configured WIP caps (#1281 step 1)", () => {
  test("configuration-only cap changes flip project and epic violations", () => {
    const board = [boardItem(1), boardItem(2), boardItem(3, "done")];
    const pool = [poolItem(1), poolItem(2), poolItem(3)];
    const config: WipCapConfig = {
      defaultCap: 10,
      laneCaps: { "in-progress": 10 },
      projectCaps: { [PROJECT]: { "in-progress": 1 } },
      epicCaps: { [EPIC]: { "in-progress": 1 } },
    };

    const initial = summarizeWipLanes(board, pool, config)[0];
    expect(initial).toMatchObject({ lane: "in-progress", count: 2, cap: 10, overCap: false });
    expect(initial.projects[0]).toMatchObject({ key: PROJECT, count: 2, cap: 1, overCap: true });
    expect(initial.epics[0]).toMatchObject({ key: EPIC, count: 2, cap: 1, overCap: true });

    const changed: WipCapConfig = {
      ...config,
      projectCaps: { [PROJECT]: { "in-progress": 2 } },
      epicCaps: { [EPIC]: { "in-progress": 2 } },
    };
    const updated = summarizeWipLanes(board, pool, changed)[0];
    expect(updated.projects[0].overCap).toBe(false);
    expect(updated.epics[0].overCap).toBe(false);
  });

  test("board headers and Today show lane, project, and epic violations; done is excluded", () => {
    const cap = WIP_CAPS.laneCaps["in-progress"];
    const board = Array.from({ length: cap + 1 }, (_, i) => boardItem(i + 1));
    const pool = board.map((_, i) => poolItem(i + 1));
    const poolByRef = new Map(pool.map((entry) => [entry.ref, entry]));
    const boardHtml = renderBoardBody(board, new Map(), undefined, { poolByRef });
    expect(boardHtml).toContain(`in-progress (${cap + 1}/${cap})`);
    expect(boardHtml).toContain(`lane over cap by 1`);
    expect(boardHtml).toContain(`project ${PROJECT} ${cap + 1}/${cap}`);
    expect(boardHtml).toContain(`epic ${EPIC} ${cap + 1}/${cap}`);

    const today = deriveToday(board, [], new Map(), null, new Date("2026-11-03T12:00:00Z"), undefined, pool);
    expect(today.wipLanes?.find((lane) => lane.lane === "in-progress")?.overCap).toBe(true);
    const todayHtml = renderTodayBody(today, buildAttention([], null), emptyQuota, []);
    expect(todayHtml).toContain("WIP cap violations:");
    expect(todayHtml).toContain(`project ${PROJECT} · in-progress ${cap + 1}/${cap}`);
    expect(todayHtml).toContain(`epic ${EPIC} · in-progress ${cap + 1}/${cap}`);
    expect(todayHtml).toContain("Informational only; work is not blocked.");
  });
});
