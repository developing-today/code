import { buildEpicRollups } from "/home/user/git/fleet/5898-3954-probe/pm-stack/aggregator/src/epics.ts";
import { buildRoadmapDrill, renderRoadmap } from "/home/user/git/fleet/5898-3954-probe/pm-stack/aggregator/src/render.ts";
import { reconcileRoadmap } from "/home/user/git/fleet/5898-3954-probe/pm-stack/aggregator/src/roadmap-reconcile.ts";

// Same shape as test/epics.test.ts fixture: child #3 is status "Done" but state "open".
function item(partial: any): any {
  return {
    id: `PVTI_${partial.number}`,
    url: "https://github.com/orgs/developing-today/projects/4?pane=issue&itemId=1",
    projectUrl: "https://github.com/orgs/developing-today/projects/4",
    ref: `developing-today/code#${partial.number}`,
    repo: "developing-today/code",
    issueUrl: `https://github.com/developing-today/code/issues/${partial.number}`,
    title: `item ${partial.number}`,
    state: "open",
    contentKind: "issue",
    status: null,
    area: "pm-dashboard",
    horizon: null,
    kind: null,
    ...partial,
  };
}
const R = (n: number) => `developing-today/code#${n}`;
const pool = [
  item({ number: 1, title: "Epic one", kind: "Epic" }),
  item({ number: 2, epicRef: R(1), status: "In Progress", blockedBy: [R(99)] }),
  item({ number: 3, epicRef: R(1), status: "Done", state: "open" }),
  item({ number: 4, epicRef: R(1), status: "Todo", blockedBy: [R(5)] }),
  item({ number: 5, state: "closed" }),
  item({ number: 6, epicRef: R(1), state: "merged", status: "No longer used" }),
];

const rollup = buildEpicRollups(pool).get(R(1) as any);
console.log("epic rollup (/projects, /program, /item):", JSON.stringify(rollup));

const row: any = {
  id: 1, subject: "grist:doc/roadmap/1", title: "Epic one", area: "id",
  status: "In Progress", horizon: "Quarter", source: "grist", notes: "",
  links: [{ ref: R(1), relation: "tracks" }],
};
const rec = reconcileRoadmap([row], pool as any);
console.log("reconciliation matched:", JSON.stringify(rec.matched.map((m: any) => [m.row.subject, m.epic?.ref])));
const drill = buildRoadmapDrill([row], rec, pool as any);
const d = drill.get("grist:doc/roadmap/1");
console.log("drill children:", d?.children.map((c: any) => `${c.ref} state=${c.state} status=${c.status}`));

const PROV: any = { board: "x", roadmap: "x", items: "x", comments: "x", quota: "x", usage: "x", attention: "x", orchestration: "x", dns: "x", registry: "x", aa: "x" };
const html = renderRoadmap([row], new Map(), PROV, "quarter", undefined, undefined, rec, drill);
const m = html.match(/Progress: (\d+)\/(\d+) child issues done/);
console.log("roadmap drill-down progress:", m ? `${m[1]}/${m[2]}` : "NOT FOUND");
console.log(rollup && m && (Number(m[1]) !== rollup.done || Number(m[2]) !== rollup.total)
  ? "DIVERGENCE CONFIRMED: same epic, same pool, two done-counts"
  : "no divergence");
