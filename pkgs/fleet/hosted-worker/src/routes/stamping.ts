// Stamping door — route module stub (#12583 skeleton).
// Filled by #12585: Metadata parse/stamp/DB insert with the fleet App
// installation-token rotation held server-side (design §3.1, §5).

import type { RouteDef } from "../router.ts";
import { stubHandler } from "./stub.ts";

export const routes: RouteDef[] = [
  { method: "POST", path: "/v1/stamps", door: "stamping", handler: stubHandler("stamping", "#12585") },
];
