#!/usr/bin/env python3
"""Adapt the copied desktop-commander Worker package into the mcpx Worker.
Run from the worktree root (~/git/code-mcpx-door-20261123) AFTER:
  cp -r pkgs/desktop-commander/worker/. pkgs/mcpx/worker/ (minus node_modules)
  cp .github/workflows/desktop-commander-worker-{publish,promote,tests}.yml
     .github/workflows/mcpx-worker-{publish,promote,tests}.yml
Idempotent-ish: asserts anchors before replacing; safe to inspect, not to re-run blindly.
"""
import json, pathlib, sys

ROOT = pathlib.Path.cwd()
PKG = ROOT / "pkgs/mcpx/worker"
assert (PKG / "src/index.ts").exists(), "run from worktree root with the copy in place"

FILTER_TS = r'''// Per-request filtering seam for the mcpx Worker's /mcp proxy
// (#13657).
//
// ARCHITECTURE ONLY: the enforcement spec (which tools/resources a
// given selector admits) lands separately. Until then this module is
// the one place where filtering decisions will live, and the proxy
// passes every response through filterMcpResponse unchanged.
//
// Selector model:
//   - The client picks a selector with the `ns` query parameter on
//     /mcp (or /), e.g. POST /mcp?ns=readonly.
//   - src/index.ts computes the selector exactly once per request via
//     resolveFilterSelector and hands it to the proxy/filter layer.
//   - The `ns` parameter is consumed by this door and stripped before
//     proxying; it is never forwarded to the bridge/daemon.
//   - No `ns` parameter means the conservative default profile below.

export const DEFAULT_FILTER_PROFILE = "conservative";

export interface FilterSelector {
  // The raw `ns` value as presented (trimmed), or null when absent.
  raw: string | null;
  // The effective profile: the raw selector when one was given,
  // otherwise DEFAULT_FILTER_PROFILE.
  profile: string;
}

export function resolveFilterSelector(url: URL): FilterSelector {
  const raw = url.searchParams.get("ns")?.trim() || null;
  return { raw, profile: raw ?? DEFAULT_FILTER_PROFILE };
}

// Filter hook for MCP responses coming back from the bridge.
// Pass-through until the enforcement spec lands; the selector is
// threaded through now so enforcement can be added here without
// touching the router or the proxy.
export function filterMcpResponse(
  response: Response,
  selector: FilterSelector,
): Response {
  void selector;
  return response;
}
'''

def sub(path, old, new, count=1):
    text = path.read_text()
    assert old in text, f"anchor missing in {path}: {old[:80]!r}"
    path.write_text(text.replace(old, new, count))

# ---------------------------------------------------------------- index.ts
idx = PKG / "src/index.ts"
sub(idx, """//   bridge door -> Cloudflare Tunnel -> supergateway on the laptop
//     -> the packaged Desktop Commander (stdio) on 127.0.0.1:8931.
//
// The /mcp proxy is a transparent streamable-HTTP proxy: method, path,
// query, headers and body pass through, and the response (including
// the Mcp-Session-Id header and any SSE stream) passes back untouched.""",
"""//   bridge door -> sandbox-link Cloudflare Tunnel ->
//     http://localhost:8932 bridge proxy on the laptop -> the mcpx
//     daemon's /mcp endpoint, discovered via the daemon state
//     descriptor. The daemon is loopback-only and unauthenticated;
//     this door is its security boundary, and /v1/* is never proxied.
//
// Bare-path equivalence: requests to / are handled exactly as /mcp —
// same auth gate, same proxying (the upstream path is always /mcp).
//
// The /mcp proxy is a transparent streamable-HTTP proxy: method,
// headers and body pass through, and the response (including the
// Mcp-Session-Id header and any SSE stream) passes back untouched.
// The one query parameter the door consumes itself is `ns`: it is
// parsed once, in the router, into a per-request filter selector
// (src/filter.ts), stripped before proxying, and handed to the
// proxy/filter layer; every other query parameter passes through.""")

sub(idx, '} from "./oauth";', '} from "./oauth";\nimport {\n  filterMcpResponse,\n  resolveFilterSelector,\n  type FilterSelector,\n} from "./filter";')

old_tail_start = '    if (path !== "/mcp") {'
text = idx.read_text()
start = text.index(old_tail_start)
end_marker = '    const upstream = await fetch(target, init);'
end = text.index(end_marker)
tail_old = text[start:end]
assert 'return json({ error: "not found" }, 404);' in tail_old
assert "isAuthorizedMcpCaller" in tail_old
new_tail = '''    // Bare-path equivalence: / is the same door as /mcp.
    if (path !== "/mcp" && path !== "/") {
      return json({ error: "not found" }, 404);
    }

    if (!(await isAuthorizedMcpCaller(request, env))) {
      return new Response(JSON.stringify({ error: "unauthorized" }), {
        status: 401,
        headers: {
          "content-type": "application/json",
          "WWW-Authenticate":
            'Bearer resource_metadata="https://commander.security.cab/.well-known/oauth-protected-resource"',
        },
      });
    }
    if (!env.ACCESS_CLIENT_ID || !env.ACCESS_CLIENT_SECRET) {
      return json({ error: "bridge credentials are not configured" }, 500);
    }

    // The filter selector is computed in exactly one place — here,
    // from the `ns` query parameter — and handed to the proxy/filter
    // layer (src/filter.ts).
    const selector = resolveFilterSelector(url);
    return proxyMcp(request, env, selector);
  },
};

// Transparent streamable-HTTP proxy to the bridge door. The upstream
// path is always /mcp (bare-path equivalence), the caller's own
// credentials are swapped for the Worker's bridge egress token, and
// the `ns` selector parameter is consumed here, never forwarded.
export async function proxyMcp(
  request: Request,
  env: Env,
  selector: FilterSelector,
): Promise<Response> {
  const url = new URL(request.url);
  const upstreamParams = new URLSearchParams(url.search);
  upstreamParams.delete("ns");
  const query = upstreamParams.toString();
  const target = new URL("/mcp" + (query ? `?${query}` : ""), env.BRIDGE_BASE);
  const headers = new Headers(request.headers);
  headers.delete("host");
  // The caller's own credentials are for the public door only; the
  // bridge door gets the Worker's egress token instead.
  headers.delete("authorization");
  headers.delete("cf-access-jwt-assertion");
  headers.delete("cf-access-client-id");
  headers.delete("cf-access-client-secret");
  headers.set("CF-Access-Client-Id", env.ACCESS_CLIENT_ID);
  headers.set("CF-Access-Client-Secret", env.ACCESS_CLIENT_SECRET);

  const init: RequestInit = {
    method: request.method,
    headers,
    redirect: "manual",
  };
  if (request.method !== "GET" && request.method !== "HEAD") {
    init.body = request.body;
  }

'''
text = text[:start] + new_tail + text[end:]
# the original closing of fetch handler + object now follows the upstream
# return; rewrite that ending into the proxyMcp ending
old_ending = '''    const upstream = await fetch(target, init);
    return new Response(upstream.body, {
      status: upstream.status,
      statusText: upstream.statusText,
      headers: new Headers(upstream.headers),
    });
  },
};
'''
new_ending = '''  const upstream = await fetch(target, init);
  const response = new Response(upstream.body, {
    status: upstream.status,
    statusText: upstream.statusText,
    headers: new Headers(upstream.headers),
  });
  return filterMcpResponse(response, selector);
}
'''
assert old_ending in text, "index.ts ending anchor missing"
text = text.replace(old_ending, new_ending)
idx.write_text(text)

# ---------------------------------------------------------------- filter.ts
(PKG / "src/filter.ts").write_text(FILTER_TS)

# ---------------------------------------------------------------- global swaps
SWAPS = [
    ("desktop-commander-worker-publish.yml", "mcpx-worker-publish.yml"),
    ("desktop-commander-worker-promote.yml", "mcpx-worker-promote.yml"),
    ("desktop-commander-worker-tests.yml", "mcpx-worker-tests.yml"),
    ("pkgs/desktop-commander/worker", "pkgs/mcpx/worker"),
    ("desktop-commander-production", "mcpx-production"),
    ("DESKTOP_COMMANDER_WORKER_CF_TOKEN", "MCPX_WORKER_CF_TOKEN"),
    ("DC_WORKER_", "MCPX_WORKER_"),
    ("desktop-commander-worker", "mcpx-worker"),
    ("desktop-commander-link", "mcpx-link"),
    ("commander-link.security.cab", "mcpx-link.security.cab"),
    ("commander.security.cab", "mcpx.security.cab"),
    ("Desktop Commander", "mcpx"),
    ("desktop-commander", "mcpx"),
    ('"commander"', '"mcpx"'),
    ("c6ddf0a5c0ba8742c0a3c74d12495bf5c4fe9bf6f01f604d62583cb2ed353879",
     "b34b9764fb317774bf5c0ff0ac9cbd8a2ab80ed346d06f08d74967011bac640e"),
    ("ea675d3c7d5b4f3b9f1cecae82e57d1b", "11eb93dc9fa24087ae57b12483dc6172"),
    ("issues/13404", "issues/13657"),
    ("#13404", "#13657"),
    ("`dc-${sha", "`mcpx-${sha"),
]
targets = [p for p in PKG.rglob("*") if p.is_file()
           and p.name not in ("package-lock.json", "README.md")
           and "node_modules" not in p.parts]
targets += [ROOT / ".github/workflows/mcpx-worker-publish.yml",
            ROOT / ".github/workflows/mcpx-worker-promote.yml",
            ROOT / ".github/workflows/mcpx-worker-tests.yml"]
for p in targets:
    t = p.read_text()
    orig = t
    for a, b in SWAPS:
        t = t.replace(a, b)
    if t != orig:
        p.write_text(t)

lock = PKG / "package-lock.json"
lt = lock.read_text().replace('"name": "desktop-commander-worker"', '"name": "mcpx-worker"')
lock.write_text(lt)

# ---------------------------------------------------------------- proxy tests
pt = PKG / "test/proxy.test.ts"
sub(pt, 'import worker, { type Env } from "../src/index";',
        'import worker, { type Env } from "../src/index";\nimport { DEFAULT_FILTER_PROFILE, resolveFilterSelector } from "../src/filter";')
with pt.open("a") as f:
    f.write(r'''

function bareRequest(path: string, authorization?: string): Request {
  const headers: Record<string, string> = {
    "content-type": "application/json",
    accept: "application/json, text/event-stream",
  };
  if (authorization) headers["authorization"] = authorization;
  return new Request(`https://mcpx.security.cab${path}`, {
    method: "POST",
    headers,
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/list" }),
  });
}

function stubBridge() {
  const bridge = vi.fn(async (_url: unknown, _init?: unknown) => {
    return new Response("event: message\ndata: {}\n\n", {
      status: 200,
      headers: { "content-type": "text/event-stream", "mcp-session-id": "session-123" },
    });
  });
  vi.stubGlobal("fetch", bridge);
  return bridge;
}

describe("bare-path equivalence", () => {
  it("refuses / without a Bearer exactly like /mcp", async () => {
    const bridge = vi.fn();
    vi.stubGlobal("fetch", bridge);
    const res = await worker.fetch(bareRequest("/"), env);
    expect(res.status).toBe(401);
    expect(res.headers.get("www-authenticate")).toBe(
      'Bearer resource_metadata="https://mcpx.security.cab/.well-known/oauth-protected-resource"',
    );
    expect(bridge).not.toHaveBeenCalled();
  });

  it("proxies / to the bridge /mcp with the egress token", async () => {
    const bridge = stubBridge();
    const res = await worker.fetch(bareRequest("/", "Bearer machine-bearer-for-tests"), env);
    expect(res.status).toBe(200);
    expect(bridge).toHaveBeenCalledTimes(1);
    const [calledUrl] = bridge.mock.calls[0] as unknown as [URL, RequestInit];
    expect(String(calledUrl)).toBe("https://mcpx-link.security.cab/mcp");
  });

  it("consumes the ns selector and forwards other query parameters", async () => {
    const bridge = stubBridge();
    const res = await worker.fetch(
      bareRequest("/mcp?ns=readonly&x=1", "Bearer machine-bearer-for-tests"),
      env,
    );
    expect(res.status).toBe(200);
    const [calledUrl] = bridge.mock.calls[0] as unknown as [URL, RequestInit];
    expect(String(calledUrl)).toBe("https://mcpx-link.security.cab/mcp?x=1");
  });

  it("bare / with a selector still lands on upstream /mcp", async () => {
    const bridge = stubBridge();
    const res = await worker.fetch(
      bareRequest("/?ns=readonly", "Bearer machine-bearer-for-tests"),
      env,
    );
    expect(res.status).toBe(200);
    const [calledUrl] = bridge.mock.calls[0] as unknown as [URL, RequestInit];
    expect(String(calledUrl)).toBe("https://mcpx-link.security.cab/mcp");
  });
});

describe("filter selector", () => {
  it("defaults to the named conservative profile without ns", () => {
    const selector = resolveFilterSelector(new URL("https://mcpx.security.cab/mcp"));
    expect(selector.raw).toBeNull();
    expect(selector.profile).toBe(DEFAULT_FILTER_PROFILE);
    expect(DEFAULT_FILTER_PROFILE).toBe("conservative");
  });

  it("uses the ns value as the profile when present", () => {
    const selector = resolveFilterSelector(new URL("https://mcpx.security.cab/mcp?ns=readonly"));
    expect(selector.raw).toBe("readonly");
    expect(selector.profile).toBe("readonly");
  });
});
''')

# ---------------------------------------------------------------- checks
cfg = json.loads((PKG / "config.json").read_text())
assert cfg == {"compatibilityDate": "2025-09-15", "routes": ["mcpx.security.cab"],
               "worker": "mcpx", "workersDev": False}, cfg
for name in ("wrangler.toml", "cloudflare.config.ts"):
    t = (PKG / name).read_text()
    assert "b34b9764fb317774bf5c0ff0ac9cbd8a2ab80ed346d06f08d74967011bac640e" in t, name
    assert "11eb93dc9fa24087ae57b12483dc6172" in t, name
    assert "https://mcpx-link.security.cab" in t, name
pkg = json.loads((PKG / "package.json").read_text())
assert pkg["name"] == "mcpx-worker", pkg["name"]
assert pkg["devDependencies"]["cf"] == "1.0.0-beta.20"
assert pkg["devDependencies"]["wrangler"] == "4.152.0"
assert pkg["devDependencies"]["@cloudflare/workers-types"] == "^5.20261019.1"
for p in targets:
    t = p.read_text()
    assert "desktop-commander" not in t, f"leftover in {p}"
    assert "commander.security.cab" not in t.replace("mcpx.security.cab", ""), f"leftover domain in {p}"
oauth = (PKG / "src/oauth.ts").read_text()
assert 'ISSUER = "https://mcpx.security.cab"' in oauth
assert 'RESOURCE = "https://mcpx.security.cab/mcp"' in oauth
assert 'SCOPE = "mcpx"' in oauth
assert 'OWNER_EMAIL = "drewrypope@gmail.com"' in oauth
print("adapt.py: all substitutions + checks OK")
print("NOTE: install README.md from the companion file (workspace mcpx-door-20261123/README.md) over pkgs/mcpx/worker/README.md")
