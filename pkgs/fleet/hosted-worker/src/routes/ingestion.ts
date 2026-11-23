// Ingestion door — route module stub (#12583 skeleton).
// Filled by #12590: the GitHub webhook receiver + scheduled sweeper
// that heal create-path insert failures (design §7.3).

import type { RouteDef } from "../router.ts";
import { stubHandler } from "./stub.ts";

export const routes: RouteDef[] = [
  {
    method: "POST",
    path: "/v1/ingestion/github",
    door: "ingestion",
    handler: stubHandler("ingestion", "#12590"),
  },
];
