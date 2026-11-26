// Hosted data-plane front door (#5199).
//
// Resolves an already-authenticated account/binding to its per-user
// Durable Object. Authentication and the binding ceremony live upstream
// (pkgs/id/src/hosted_binding.rs); this worker consumes their result as
// the resolved context headers X-Binding-Id and X-Jurisdiction, which
// in deployment are set by the authenticated internal caller — the same
// trust posture as the design's "route context is internal/
// authenticated, never client-controlled".
//
// #15479: that internal caller does not exist yet (the registry that
// would resolve an identity to a binding is #15480), so any
// client-supplied binding context is a forgery by definition. The
// front door refuses it at the public edge with an explicit error and
// otherwise fails closed: binding and jurisdiction resolve
// server-side only, never from client headers. It strips every
// client-supplied internal header, validates the resolved context,
// and routes only through the jurisdiction-restricted namespace
// (routing.ts).
// No Iroh QUIC runs inside the Worker (issue excludes).

import { FailClosed, parseBindingId, parseJurisdiction, userStub, type Jurisdiction, type ObjectNamespaceLike } from "./routing.ts";
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
  USER_CEILING_BYTES?: string;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

/** Read-only routing guard (#3399): resolve the binding to a
 * validated stable record. Never signup, inventory, provision,
 * enable, or migrate here — reads only. Missing, legacy-only,
 * mismatched, and corrupt records all fail closed before any
 * namespace access.
 *
 * Unreachable from the public edge until the authenticated
 * internal caller lands (#15479): the front door refuses
 * client-supplied binding context before parsing, so only
 * server-resolved context reaches this guard. Unit-tested
 * directly; the front-door tests assert the edge refusal. */
export async function assertDirectoryBinding(
  bindingId: string,
  jurisdiction: Jurisdiction,
  db: HostedDirectoryDatabase,
): Promise<void> {
  const directory = new PersistentHostedAccountDirectory(db);
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

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    try {
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
      // #15479: no authenticated internal caller exists yet (its
      // registry is #15480), so any client-supplied binding context
      // is refused at the public edge instead of routed on — the
      // fleet-hosted posture: internal context is never
      // client-controlled. Client input is not even parsed here; a
      // malformed forgery and a well-formed one refuse identically.
      if (req.headers.has("x-binding-id") || req.headers.has("x-jurisdiction")) {
        throw new FailClosed(
          401,
          "client_binding_refused",
          "client-supplied binding context is refused at the public edge; binding and jurisdiction resolve server-side only (#15479)",
        );
      }
      const bindingId = parseBindingId(req.headers.get("x-binding-id"));
      const jurisdiction = parseJurisdiction(req.headers.get("x-jurisdiction"));
      if (env.HOSTED_DIRECTORY) {
        await assertDirectoryBinding(bindingId, jurisdiction, env.HOSTED_DIRECTORY);
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
