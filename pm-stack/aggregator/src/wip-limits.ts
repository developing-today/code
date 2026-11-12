import wipCaps from "./wip-caps.json";
import { applyBoardDonePolicy } from "./board-policy.ts";
import type { BoardItem, ProjectPoolItem } from "./model.ts";

export interface WipCapConfig {
  defaultCap: number;
  laneCaps: Record<string, number>;
  /** Overrides keyed by the GitHub Project URL carried in the pool. */
  projectCaps: Record<string, Record<string, number>>;
  /** Overrides keyed by the canonical epic issue ref. */
  epicCaps: Record<string, Record<string, number>>;
}

export interface WipScopeCount {
  key: string;
  count: number;
  cap: number;
  overCap: boolean;
}

export interface WipLaneSummary {
  lane: string;
  count: number;
  cap: number;
  overCap: boolean;
  projects: WipScopeCount[];
  epics: WipScopeCount[];
}

export const WIP_CAPS = wipCaps as WipCapConfig;

/** Count visible board work by lane, GitHub Project, and epic. Done is
 * completion history, not WIP. Project/epic caps inherit the lane cap
 * unless the single configuration file provides an override. */
export function summarizeWipLanes(
  board: BoardItem[],
  projectPool: ProjectPoolItem[] = [],
  config: WipCapConfig = WIP_CAPS,
): WipLaneSummary[] {
  const visible = applyBoardDonePolicy(board).filter((item) => item.status !== "done");
  const poolByRef = new Map(projectPool.map((item) => [item.ref, item]));
  const lanes = new Map<string, { items: BoardItem[]; projects: Map<string, number>; epics: Map<string, number> }>();
  for (const lane of Object.keys(config.laneCaps)) {
    lanes.set(lane, { items: [], projects: new Map(), epics: new Map() });
  }

  for (const item of visible) {
    const lane = item.status === "to-do" ? "to-do" : item.status;
    const summary = lanes.get(lane) ?? {
      items: [] as BoardItem[],
      projects: new Map<string, number>(),
      epics: new Map<string, number>(),
    };
    summary.items.push(item);
    const projects = new Set<string>();
    const epics = new Set<string>();
    for (const link of item.links) {
      const pooled = poolByRef.get(link.ref);
      if (!pooled) continue;
      projects.add(pooled.projectUrl);
      if (pooled.epicRef) epics.add(pooled.epicRef);
    }
    for (const project of projects) summary.projects.set(project, (summary.projects.get(project) ?? 0) + 1);
    for (const epic of epics) summary.epics.set(epic, (summary.epics.get(epic) ?? 0) + 1);
    lanes.set(lane, summary);
  }

  const capFor = (lane: string): number => config.laneCaps[lane] ?? config.defaultCap;
  const scopes = (counts: Map<string, number>, overrides: Record<string, Record<string, number>>, lane: string) =>
    [...counts]
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([key, count]) => {
        const cap = overrides[key]?.[lane] ?? capFor(lane);
        return { key, count, cap, overCap: count > cap };
      });

  return [...lanes]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([lane, summary]) => {
      const count = summary.items.length;
      const cap = capFor(lane);
      return {
        lane,
        count,
        cap,
        overCap: count > cap,
        projects: scopes(summary.projects, config.projectCaps, lane),
        epics: scopes(summary.epics, config.epicCaps, lane),
      };
    });
}
