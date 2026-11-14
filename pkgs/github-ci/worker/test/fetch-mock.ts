// In-isolate replacement for the `fetchMock` export that
// @cloudflare/vitest-pool-workers removed in 0.23 (it was built on a
// workerd-internal hook that no longer exists). The tests in this
// package run the worker in the same isolate — they import the handler
// and call it with createExecutionContext, and the Turso client calls
// the global fetch — so patching globalThis.fetch reproduces the
// semantics the tests rely on:
//   - interceptors are one-shot and consumed in registration order,
//   - disableNetConnect makes any unmocked fetch throw,
//   - assertNoPendingInterceptors fails when a registered reply was
//     never consumed.
// Only the subset of the undici MockAgent API the tests use is
// implemented: get(origin).intercept({ path, method }).reply(status,
// body | (opts) => body, { headers }?).

type ReplyBody = string | ((opts: { body: string }) => string);

interface Interceptor {
  origin: string;
  path: string;
  method: string;
  status: number;
  body: ReplyBody;
  headers?: Record<string, string>;
}

const pending: Interceptor[] = [];
let netConnectDisabled = false;
let active = false;
let realFetch: typeof fetch | undefined;

class MockPool {
  constructor(private origin: string) {}

  intercept(opts: { path: string; method?: string }) {
    const origin = this.origin;
    return {
      reply(status: number, body: ReplyBody, replyOpts?: { headers?: Record<string, string> }) {
        pending.push({
          origin,
          path: opts.path,
          method: (opts.method ?? "GET").toUpperCase(),
          status,
          body,
          headers: replyOpts?.headers,
        });
      },
    };
  }
}

async function mockedFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const req = new Request(input as RequestInfo, init);
  const url = new URL(req.url);
  const idx = pending.findIndex(
    (i) => i.origin === url.origin && i.path === url.pathname && i.method === req.method.toUpperCase(),
  );
  if (idx === -1) {
    if (netConnectDisabled) {
      throw new Error(`fetchMock: net connect disabled and no interceptor matched ${req.method} ${req.url}`);
    }
    if (!realFetch) throw new Error("fetchMock: not activated");
    return realFetch(input, init);
  }
  const [hit] = pending.splice(idx, 1);
  const reqBody = req.method === "GET" || req.method === "HEAD" ? "" : await req.text();
  const body = typeof hit.body === "function" ? hit.body({ body: reqBody }) : hit.body;
  return new Response(body, { status: hit.status, headers: hit.headers });
}

export const fetchMock = {
  activate() {
    if (active) return;
    active = true;
    realFetch = globalThis.fetch;
    globalThis.fetch = mockedFetch as typeof fetch;
  },
  deactivate() {
    if (!active) return;
    active = false;
    if (realFetch) globalThis.fetch = realFetch;
    pending.length = 0;
  },
  disableNetConnect() {
    netConnectDisabled = true;
  },
  enableNetConnect() {
    netConnectDisabled = false;
  },
  get(origin: string) {
    return new MockPool(origin);
  },
  assertNoPendingInterceptors() {
    if (pending.length > 0) {
      const left = pending.map((i) => `${i.method} ${i.origin}${i.path}`).join(", ");
      pending.length = 0;
      throw new Error(`fetchMock: interceptor(s) registered but never consumed: ${left}`);
    }
  },
};
