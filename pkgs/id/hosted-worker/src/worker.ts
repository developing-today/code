// Hosted data-plane front door (#5199).
//
// Resolves an already-authenticated account/binding to its per-user
// Durable Object. Authentication and the binding ceremony live upstream
// (pkgs/id/src/hosted_binding.rs); this worker consumes their result as
// the resolved context headers X-Binding-Id and X-Jurisdiction, which
// in deployment are set by the authenticated internal caller — the same
// trust posture as the design's "route context is internal/
// authenticated, never client-controlled". It strips every client-
// supplied internal header, validates the resolved context, and routes
// only through the jurisdiction-restricted namespace (routing.ts).
// No Iroh QUIC runs inside the Worker (issue excludes).

import { FailClosed, parseBindingId, parseJurisdiction, userStub, type ObjectNamespaceLike } from "./routing.ts";
import { TenancyRegistry, type TenancyDatabase } from "./tenancy-registry.ts";
import { PersistentHostedAccountDirectory, type HostedDirectoryDatabase } from "./persistent-account-directory.ts";
import { HDR_BINDING, HDR_CSE, HDR_IROH, HDR_JURISDICTION, HDR_ROUTED, bodyTooLarge, requestBodyLimit } from "./user-do.ts";
import { FENCE_RETRY_AFTER_SECONDS, HDR_MIGRATION } from "./migration-fence.ts";

export { UserDO } from "./user-do.ts";
// The tenancy control plane's per-tenant operations object (E2 of
// #9083, #12797). Exported and bound so the platform knows the
// class; the front door does not route to it — the tenancy Worker
// drives it over internal calls.
export { TenantOpsDO } from "./tenant-ops.ts";

interface Env {
  USERS: ObjectNamespaceLike;
  /** The central account/binding registry. Missing configuration fails closed. */
  TENANCY_DB?: TenancyDatabase;
  /** Transitional route projection; consulted only for migration state. */
  HOSTED_DIRECTORY?: HostedDirectoryDatabase;
  USER_CEILING_BYTES?: string;
}

function json(body: unknown, status = 200, extra: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", ...extra },
  });
}

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    try {
      const bindingId = parseBindingId(req.headers.get("x-binding-id"));
      const jurisdiction = parseJurisdiction(req.headers.get("x-jurisdiction"));
      const binding = await new TenancyRegistry(env.TENANCY_DB).binding(bindingId);
      if (binding === null) {
        throw new FailClosed(403, "registry_binding_missing", "no central binding record exists; failing closed");
      }
      if (binding.bindingStatus !== "active") {
        throw new FailClosed(403, "registry_binding_inactive", "central binding is not active; failing closed");
      }
      if (binding.accountState !== "active") {
        throw new FailClosed(403, "registry_account_inactive", "central account is not active; failing closed");
      }
      if (env.HOSTED_DIRECTORY) {
        // The central registry decides whether the binding/account exists
        // and is active. The legacy directory is only a migration routing
        // projection; it cannot grant access without the central row.
        const directory = new PersistentHostedAccountDirectory(env.HOSTED_DIRECTORY);
        try {
          const routeRecord = await directory.findByBindingId(bindingId);
          if (routeRecord) {
            if (await directory.legacyInventory(routeRecord.subjectKey) || await directory.findLegacyByBindingId(bindingId)) {
              throw new FailClosed(503, "invalid_directory_record", "account conflicts with legacy inventory; failing closed");
            }
            if (routeRecord.phase === "switched") {
              return json(
                { error: "migration_switching", message: "account is switching jurisdiction; retry shortly" },
                503,
                { "retry-after": String(FENCE_RETRY_AFTER_SECONDS) },
              );
            }
            if (routeRecord.jurisdiction !== jurisdiction) {
              throw new FailClosed(403, "directory_jurisdiction_mismatch", "request jurisdiction does not match the migration route projection; failing closed");
            }
          } else if (await directory.findLegacyByBindingId(bindingId)) {
            throw new FailClosed(
              403,
              "legacy_subject_requires_migration",
              "legacy binding has no central account record; failing closed",
            );
          }
        } catch (error) {
          if (error instanceof FailClosed) throw error;
          throw new FailClosed(503, "directory_unavailable", "migration route projection unavailable; failing closed");
        }
      }
      const hasBody = req.method !== "GET" && req.method !== "HEAD";
      if (hasBody) {
        // Front-door memory bound (#11337): refuse a declared
        // oversized body before anything reads it. The body is then
        // streamed, never buffered here, so the DO's capped reads
        // bound memory for chunked bodies too.
        const limit = requestBodyLimit(env.USER_CEILING_BYTES);
        const declared = req.headers.get("content-length");
        if (declared !== null && Number(declared) > limit) throw bodyTooLarge(limit);
      }
      const stub = userStub(env.USERS, bindingId, jurisdiction);
      const headers = new Headers(req.headers);
      headers.delete("x-binding-id");
      headers.delete("x-jurisdiction");
      // The bound ID key (#3818): consumed from the internal
      // caller like the binding context, stripped of any
      // client-supplied copy, and re-set as an internal header.
      // Absent stays absent — the bridge routes fail closed on it.
      const irohId = req.headers.get("x-iroh-id");
      headers.delete("x-iroh-id");
      headers.delete(HDR_IROH);
      if (irohId !== null) headers.set(HDR_IROH, irohId);
      // The binding's client-side-encryption choice (§7 of the
      // #2258 spec): consumed from the internal caller like the
      // ID key — the binding record is its source — stripped of
      // any client-supplied copy under either name, and re-set
      // as an internal header. Absent stays absent; the DO binds
      // its persisted default and fails closed on disagreement.
      const cse = req.headers.get("x-client-side-encryption");
      headers.delete("x-client-side-encryption");
      headers.delete(HDR_CSE);
      if (cse !== null) headers.set(HDR_CSE, cse);
      // The migration principal (#3404) is set only by the migration
      // coordinator on its own stub calls, never through this door.
      headers.delete(HDR_MIGRATION);
      headers.set(HDR_BINDING, bindingId);
      headers.set(HDR_JURISDICTION, jurisdiction);
      headers.set(HDR_ROUTED, "restricted");
      const forwarded = new Request(req.url, {
        method: req.method,
        headers,
        body: hasBody ? (req.body ?? undefined) : undefined,
        duplex: "half",
      } as RequestInit);
      return await stub.fetch(forwarded);
    } catch (e) {
      if (e instanceof FailClosed) return json({ error: e.code, message: e.message }, e.status);
      throw e;
    }
  },
};
