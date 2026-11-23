// TEST-ONLY harness (wrangler.test.toml). Local workerd cannot run the
// application path at all — namespace.jurisdiction() throws
// "Jurisdiction restrictions are not implemented in workerd" — so the
// UserDO behaviour (bootstrap, identity guard, isolation) is exercised
// here by addressing objects directly and supplying the internal
// routing context the front door would have set. This file is never an
// application entry point; wrangler.toml does not reference it.

import { HDR_BINDING, HDR_IROH, HDR_JURISDICTION, HDR_ROUTED } from "./user-do.ts";

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
    // Routing context: the query params below, or the path prefix
    // `/__harness/<name>/<binding>/<iroh>/<jurisdiction>/…` (#3818
    // live test). A base URL cannot carry query params before the
    // path — BridgeHttpClient builds URLs as base + path — so the
    // live native client routes through the prefix while the smoke
    // (smoke.mjs part 2) keeps using the query params. Both forms
    // set the same headers and forward the remainder unchanged.
    let name = url.searchParams.get("name");
    let binding = url.searchParams.get("binding") ?? "";
    let iroh = url.searchParams.get("iroh");
    let jurisdiction = url.searchParams.get("jurisdiction") ?? "";
    let routed = url.searchParams.get("routed");
    let pathname = url.pathname;
    const prefix =
      pathname.match(/^\/__harness\/([^/]+)\/([^/]+)\/([^/]+)\/([^/]+)(\/.*)?$/);
    if (prefix) {
      name = prefix[1];
      binding = prefix[2];
      iroh = prefix[3];
      jurisdiction = prefix[4];
      pathname = prefix[5] ?? "/";
    }
    if (!name) return new Response("missing name", { status: 400 });
    const headers = new Headers(req.headers);
    headers.set(HDR_BINDING, binding);
    headers.set(HDR_JURISDICTION, jurisdiction);
    if (iroh !== null) headers.set(HDR_IROH, iroh);
    if (routed !== "no") headers.set(HDR_ROUTED, "restricted");
    const stub = env.TEST_USERS.get(env.TEST_USERS.idFromName(name));
    const forwarded = new Request("https://do" + pathname, {
      method: req.method,
      headers,
      body: req.method === "GET" || req.method === "HEAD" ? undefined : await req.arrayBuffer(),
    });
    return await stub.fetch(forwarded);
  },
};
