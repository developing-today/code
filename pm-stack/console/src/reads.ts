// Reads that need nothing but GitHub and the queue store.
//
// They are answered by the same code wherever the app is served from: a
// machine asks through the fleet binary, and Cloudflare asks GitHub's
// and the queue store's own APIs (edge/src/direct.ts). Neither needs the
// other for these. A read that does need a machine (its units, its
// agents, the dashboard's questions) is not here, and is asked of one.
import type { Ops } from "./ops.ts";
import type { Plan } from "./plan.ts";

/** The answer to `GET /console/api/<seg...>`, or undefined when that is
 * not one of these reads. */
export async function platformRead(seg: string[], query: URLSearchParams, ops: Ops, plan: Plan): Promise<unknown> {
  // `fresh=1` is the reader pressing refresh: asked again now, not
  // answered from the last minute's read.
  const fresh = query.get("fresh") === "1";
  if (seg.length === 1) {
    switch (seg[0]) {
      case "queue":
        return {
          counts: await ops.queueCounts(),
          tasks: await ops.queueList(query.get("status") ?? "queued", query.get("q"), query.get("limit")),
        };
      case "priorities":
        return ops.priorities();
      case "prs":
        return ops.prs();
      case "releases":
        return ops.releases();
      case "roadmaps":
        return plan.capabilities();
      case "roadmap":
        return plan.roadmap(query.get("area"), fresh);
      case "bugs":
        return plan.bugs(fresh);
      case "intake":
        return plan.intakeList(fresh);
    }
  }
  if (seg.length === 2 && seg[0] === "prs") return ops.pr(seg[1]);
  return undefined;
}
