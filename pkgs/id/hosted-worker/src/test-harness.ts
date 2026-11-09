// TEST-ONLY harness (wrangler.test.toml). Local workerd cannot run the
// application path at all — namespace.jurisdiction() throws
// "Jurisdiction restrictions are not implemented in workerd" — so the
// UserDO behaviour (bootstrap, identity guard, isolation) is exercised
// here by addressing objects directly and supplying the internal
// routing context the front door would have set. This file is never an
// application entry point; wrangler.toml does not reference it.

import { HDR_BINDING, HDR_JURISDICTION, HDR_ROUTED } from "./user-do.ts";

export { UserDO } from "./user-do.ts";

interface Ns {
  idFromName(name: string): unknown;
  get(id: unknown): { fetch(req: Request): Promise<Response> };
}
interface Env {
  TEST_USERS: Ns;
}

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    const url = new URL(req.url);
    const name = url.searchParams.get("name");
    if (!name) return new Response("missing name", { status: 400 });
    const headers = new Headers(req.headers);
    headers.set(HDR_BINDING, url.searchParams.get("binding") ?? "");
    headers.set(HDR_JURISDICTION, url.searchParams.get("jurisdiction") ?? "");
    if (url.searchParams.get("routed") !== "no") headers.set(HDR_ROUTED, "restricted");
    const stub = env.TEST_USERS.get(env.TEST_USERS.idFromName(name));
    const forwarded = new Request("https://do" + url.pathname, {
      method: req.method,
      headers,
      body: req.method === "GET" || req.method === "HEAD" ? undefined : await req.arrayBuffer(),
    });
    return await stub.fetch(forwarded);
  },
};
