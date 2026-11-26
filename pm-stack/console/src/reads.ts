// Reads that need nothing but GitHub, the queue store and the bridge.
//
// They are answered by the same code wherever the app is served from: a
// machine asks through the fleet binary, and Cloudflare asks GitHub's
// and the queue store's own APIs and the bridge Worker over its
// service binding (edge/src/direct.ts). Neither needs the other for
// these. A read that does need a machine (its units, the dashboard's
// questions) is not here, and is asked of one.
import type { Ops } from "./ops.ts";
import type { Plan } from "./plan.ts";
import type { Trends } from "./trends.ts";

/** The answer to `GET /console/api/<seg...>`, or undefined when that is
 * not one of these reads. */
export async function platformRead(
  seg: string[],
  query: URLSearchParams,
  ops: Ops,
  plan: Plan,
  trends?: Trends,
): Promise<unknown> {
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
      case "platform":
        return ops.platform();
      // The Cloudflare configuration's last successful apply (#13495):
      // the apply workflow's runs are GitHub's record, so this is
      // answered wherever the app is served from, like the rest here.
      case "cloudflare-apply":
        return ops.cloudflareApply();
      // The stores, asked live (#14254). Where a store's token
      // cannot be fetched (the edge holds none by name), that store
      // answers as not answering; the others still show.
      case "stores":
        return ops.platformStores();
      case "roadmaps":
        return plan.capabilities();
      case "roadmap":
        return plan.roadmap(query.get("area"), fresh);
      case "bugs":
        return plan.bugs(fresh);
      case "intake":
        return plan.intakeList(fresh);
      // The bridge's message log (#14373). On a machine this address
      // is answered by the server's own switch before this list is
      // reached; at the edge it is this list, read through the PM
      // bridge credential. (`agents` is a document section instead:
      // edge/src/direct.ts reads it the same way.)
      case "messages":
        return ops.messages(query.get("limit"));
      // The unified owner pipeline and the unified agents list
      // (#14374). Same story as `messages`: at the edge these are
      // read through the PM bridge credential (edge/src/direct.ts
      // translates the bridge argv); on a machine the server's switch
      // may answer first, and either way the Ops call is the same.
      case "pipeline":
        return ops.pipelineFeed(query);
      case "directory":
        return ops.directory();
      // Every offer with its claims (#14375), for the composer's
      // offer rows on the Messages surface. Same bridge story as the
      // pipeline: the PM credential's read scope opens it at the
      // edge, a machine reads it with the owner token.
      case "offers":
        return ops.offers();
      // The minted enroll tokens' state (#14376): the provisioning
      // half of the Agents screen. Same story again — state only,
      // never a hash or a value.
      case "enroll-tokens":
        return ops.enrollTokens();
      // The wake hooks (#14544), for the Agents screen's hook card.
      // Same bridge story again — the list is a read; setting and
      // removing are actions (actions.ts), never reads.
      case "wake-hooks":
        return ops.wakeHooks();
      case "trends":
        return trends ? trends.read(query.get("days"), fresh, query.get("end")) : undefined;
    }
  }
  if (seg.length === 2 && seg[0] === "prs") return ops.pr(seg[1]);
  return undefined;
}
