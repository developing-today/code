// Hosted data-plane front door (#5199).
//
// Authenticates the caller and resolves its account/binding to the
// per-user Durable Object. Caller authentication lives HERE
// (#15479, front-door-auth.ts): the request's bearer credential is
// resolved server-side to the one binding and jurisdiction it may
// act for, and routing uses only that resolved record. The binding
// ceremony itself lives upstream (pkgs/id/src/hosted_binding.rs).
// Client-supplied X-Binding-Id / X-Jurisdiction are never routing
// input: at most they are checked against the resolved record, and
// a mismatch is refused. Every client-supplied internal header is
// stripped, and routing goes only through the
// jurisdiction-restricted namespace (routing.ts).
// No Iroh QUIC runs inside the Worker (issue excludes).

import { FailClosed, userStub, type ObjectNamespaceLike } from "./routing.ts";
import { authenticateFrontDoor, frontDoorCallersFromEnv } from "./front-door-auth.ts";
import { PersistentHostedAccountDirectory, type HostedDirectoryDatabase } from "./persistent-account-directory.ts";
import { HDR_BINDING, HDR_CSE, HDR_IROH, HDR_JURISDICTION, HDR_ROUTED, bodyTooLarge, requestBodyLimit } from "./user-do.ts";

export { UserDO } from "./user-do.ts";
// The tenancy control plane's per-tenant operations object (E2 of
// #9083, #12797). Exported and bound so the platform knows the
// class; the front door does not route to it — the tenancy Worker
// drives it over internal calls.
export { TenantOpsDO } from "./tenant-ops.ts";

interface Env {
  USERS: ObjectNamespaceLike;
  /** Optional structural adapter. Raw D1 is not compatible: its `.all()`
   * result must be unwrapped from `{ results }` by an adapter first. */
  HOSTED_DIRECTORY?: HostedDirectoryDatabase;
  /** The front-door caller directory (#15479): a JSON array of
   * `{ token, bindingId, jurisdiction, irohId? }` records. Absent
   * means NO callers are admitted — the front door fails closed. */
  FRONT_DOOR_CALLERS?: string;
  USER_CEILING_BYTES?: string;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    try {
      // Authenticate first (#15479): the credential resolves
      // server-side to the caller's own binding and jurisdiction,
      // and those resolved values — never client headers — are the
      // routing input for everything below, the account-directory
      // guard included.
      const caller = authenticateFrontDoor(req, frontDoorCallersFromEnv(env.FRONT_DOOR_CALLERS));
      const bindingId = caller.bindingId;
      const jurisdiction = caller.jurisdiction;
      if (env.HOSTED_DIRECTORY) {
        // Read-only routing guard (#3399): resolve the binding to a
        // validated stable record. Never signup, inventory, provision,
        // enable, or migrate here — reads only. Missing, legacy-only,
        // mismatched, and corrupt records all fail closed before any
        // namespace access.
        const directory = new PersistentHostedAccountDirectory(env.HOSTED_DIRECTORY);
        let record;
        try {
          record = await directory.findByBindingId(bindingId);
        } catch (error) {
          if (error instanceof FailClosed) throw error;
          throw new FailClosed(503, "directory_unavailable", "hosted account directory unavailable; failing closed");
        }
        if (!record) {
          let legacy;
          try {
            legacy = await directory.findLegacyByBindingId(bindingId);
          } catch (error) {
            if (error instanceof FailClosed) throw error;
            throw new FailClosed(503, "directory_unavailable", "hosted account directory unavailable; failing closed");
          }
          if (legacy) {
            throw new FailClosed(
              403,
              "legacy_subject_requires_migration",
              "subject has a legacy account of unknown jurisdiction; routing cannot relabel it — migration is a separately reviewed procedure",
            );
          }
          throw new FailClosed(403, "directory_account_missing", "no validated hosted account record; failing closed");
        }
        try {
          if (await directory.legacyInventory(record.subjectKey) || await directory.findLegacyByBindingId(bindingId)) {
            throw new FailClosed(503, "invalid_directory_record", "account conflicts with legacy inventory; failing closed");
          }
        } catch (error) {
          if (error instanceof FailClosed) throw error;
          throw new FailClosed(503, "directory_unavailable", "hosted account directory unavailable; failing closed");
        }
        if (record.jurisdiction !== jurisdiction) {
          throw new FailClosed(403, "directory_jurisdiction_mismatch", "request jurisdiction does not match the persisted account; failing closed");
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
      // The caller credential (#15479) is consumed at this door;
      // it is never forwarded into the user's object.
      headers.delete("authorization");
      // The bound ID key (#3818): resolved from the authenticated
      // caller record like the binding context (#15479), stripped
      // of any client-supplied copy, and re-set as an internal
      // header. Absent stays absent — the bridge routes fail
      // closed on it.
      headers.delete("x-iroh-id");
      headers.delete(HDR_IROH);
      if (caller.irohId !== undefined) headers.set(HDR_IROH, caller.irohId);
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
