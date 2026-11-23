// Query door — route module stub (#12583 skeleton).
// Filled by #12587: the hosted Metadata query surface over the
// metadata DB (design §7.1).

import type { RouteDef } from "../router.ts";
import { stubHandler } from "./stub.ts";

export const routes: RouteDef[] = [
  { method: "GET", path: "/v1/query", door: "query", handler: stubHandler("query", "#12587") },
  { method: "POST", path: "/v1/query", door: "query", handler: stubHandler("query", "#12587") },
];
