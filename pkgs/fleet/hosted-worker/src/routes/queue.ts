// Queue door — route module stub (#12583 skeleton).
// Filled by #12589: the broker protocol over hosted transports onto
// the existing fleet-queue Turso DB — a second, peer front door, not a
// second queue (design §0, §6).

import type { RouteDef } from "../router.ts";
import { stubHandler } from "./stub.ts";

export const routes: RouteDef[] = [
  { method: "POST", path: "/v1/queue", door: "queue", handler: stubHandler("queue", "#12589") },
];
