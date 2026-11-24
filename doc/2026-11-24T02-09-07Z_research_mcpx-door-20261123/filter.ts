// Per-request filtering seam for the mcpx Worker's /mcp proxy
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
