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
import { HDR_BINDING, HDR_JURISDICTION, HDR_ROUTED, bodyTooLarge, requestBodyLimit } from "./user-do.ts";

export { UserDO } from "./user-do.ts";

interface Env {
  USERS: ObjectNamespaceLike;
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
      const bindingId = parseBindingId(req.headers.get("x-binding-id"));
      const jurisdiction = parseJurisdiction(req.headers.get("x-jurisdiction"));
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
