// Kv door (#13316; design doc/2026-11-23T12-45-00Z_design_fleet_kv §4,
// §9). The fleet-kv client's central plane: a host's `fleet kv` posts
// its Hrana v2 pipeline here (FLEET_KV_URL = https://<door>/v1/kv), and
// src/kv-door.ts checks every statement against the catalogue and the
// caller before forwarding it to the shared primary. Service tokens
// need the `kv` door in their route scope; admin statements also need
// `kv-admin` (or the kv-admin role for a person or agent caller).

import type { AuthDeps, CallerContext } from "../auth.ts";
import { FailClosed } from "../errors.ts";
import { handleKvBlobCapability, handleKvBlobPut, handleKvBlobRead } from "../kv-blobs.ts";
import { handleKvPipeline } from "../kv-door.ts";
import type { RouteDef } from "../router.ts";

function caller(ctx: CallerContext | null): CallerContext {
  if (!ctx) {
    throw new FailClosed(401, "missing_token", "the kv door requires an authenticated caller");
  }
  return ctx;
}

export const routes: RouteDef[] = [
  {
    method: "POST",
    path: "/v1/kv/v2/pipeline",
    door: "kv",
    handler: (req: Request, ctx: CallerContext | null, deps: AuthDeps) => handleKvPipeline(req, caller(ctx), deps),
  },
  // The blob upload door (#13318, src/kv-blobs.ts): service tokens need
  // the `kv-upload` door in their route scope.
  {
    method: "PUT",
    path: "/v1/kv/blobs",
    door: "kv-upload",
    handler: (req: Request, ctx: CallerContext | null, deps: AuthDeps) => handleKvBlobPut(req, caller(ctx), deps),
  },
  // The read capability mint (#13318): a reader proves it may read the
  // row before it gets a five-minute, one-object read capability.
  {
    method: "POST",
    path: "/v1/kv/blobs/capability",
    door: "kv",
    handler: (req: Request, ctx: CallerContext | null, deps: AuthDeps) =>
      handleKvBlobCapability(req, caller(ctx), deps),
  },
  // The read itself is authorized by the capability alone: no other
  // credential, so the route is public and the handler fails closed
  // (401) on a capability that does not verify or has expired.
  {
    method: "GET",
    path: "/v1/kv/blobs/read",
    door: null,
    public: true,
    handler: (req: Request, _ctx: CallerContext | null, deps: AuthDeps) => handleKvBlobRead(req, deps),
  },
];
