// In-isolate replacement for the `fetchMock` export that
// @cloudflare/vitest-pool-workers removed in 0.23 (it was built on a
// workerd-internal hook that no longer exists). Mirrors the approach
// already landed for pkgs/github-ci/worker (test/fetch-mock.ts),
// extended for this package's usage: interceptors may carry a body
// matcher, replies are callbacks returning undici-style
// { statusCode, data, responseOptions } objects, and interceptors
// can be marked .persist() so they are not consumed on use.
// Interceptors are matched in registration order; one-shot
// interceptors are consumed, persistent ones are not.
// disableNetConnect makes any unmocked fetch throw.

type ReplyObject = {
  statusCode: number;
  data: string;
  responseOptions?: { headers?: Record<string, string> };
};
type ReplyFn = (opts: { body: string }) => ReplyObject;

interface Interceptor {
  origin: string;
  path: string;
  method: string;
  bodyMatcher?: (body: string) => boolean;
  replyFn: ReplyFn;
  persist: boolean;
}

const interceptors: Interceptor[] = [];
let netConnectDisabled = false;
let active = false;
let realFetch: typeof fetch | undefined;

class MockPool {
  constructor(private origin: string) {}

  intercept(opts: { path: string; method?: string; body?: (body: string) => boolean }) {
    const origin = this.origin;
    return {
      reply(fn: ReplyFn) {
        const it: Interceptor = {
          origin,
          path: opts.path,
          method: (opts.method ?? "GET").toUpperCase(),
          bodyMatcher: opts.body,
          replyFn: fn,
          persist: false,
        };
        interceptors.push(it);
        return {
          persist() {
            it.persist = true;
            return this;
          },
        };
      },
    };
  }
}

async function mockedFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const req = new Request(input as RequestInfo, init);
  const url = new URL(req.url);
  const reqBody = req.method === "GET" || req.method === "HEAD" ? "" : await req.text();
  const idx = interceptors.findIndex(
    (i) =>
      i.origin === url.origin &&
      i.path === url.pathname &&
      i.method === req.method.toUpperCase() &&
      (i.bodyMatcher === undefined || i.bodyMatcher(reqBody)),
  );
  if (idx === -1) {
    if (netConnectDisabled) {
      throw new Error(`fetchMock: net connect disabled and no interceptor matched ${req.method} ${req.url}`);
    }
    if (!realFetch) throw new Error("fetchMock: not activated");
    return realFetch(input, init);
  }
  const hit = interceptors[idx];
  if (!hit.persist) interceptors.splice(idx, 1);
  const out = hit.replyFn({ body: reqBody });
  return new Response(out.data, { status: out.statusCode, headers: out.responseOptions?.headers });
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
    interceptors.length = 0;
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
    const left = interceptors
      .filter((i) => !i.persist)
      .map((i) => `${i.method} ${i.origin}${i.path}`)
      .join(", ");
    if (left) throw new Error(`fetchMock: interceptor(s) registered but never consumed: ${left}`);
  },
};
