// Sync door — route module stub (#12583 skeleton).
// Filled by #12591: project sync + queue/GitHub sync as hosted verbs
// (design §7.2).

import type { RouteDef } from "../router.ts";
import { stubHandler } from "./stub.ts";

export const routes: RouteDef[] = [
  { method: "POST", path: "/v1/sync/project", door: "sync", handler: stubHandler("sync", "#12591") },
  {
    method: "POST",
    path: "/v1/sync/queue-github",
    door: "sync",
    handler: stubHandler("sync", "#12591"),
  },
];
