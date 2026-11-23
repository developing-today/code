// Per-request filtering for the mcpx Worker's /mcp proxy (#13657).
//
// The mcpx upstream offers no per-client hooks (X-Mcpx-Settings is
// ignored on /mcp and no per-session state exists), so this Worker
// polices every request and response itself. Enforcement is a pure
// function of (request URL selector, message): nothing is keyed to
// Mcp-Session-Id.
//
// Selector model:
//   - The client picks a selector with the `ns` query parameter on
//     /mcp (or /), e.g. POST /mcp?ns=readonly.
//   - src/index.ts computes the selector exactly once per request via
//     resolveFilterSelector and hands it to the proxy/filter layer.
//   - The `ns` parameter is consumed by this door and stripped before
//     proxying; it is never forwarded to the bridge/daemon.
//
// Profiles:
//   - no `ns`              -> "conservative", selected set is empty.
//   - `ns=a,b`             -> "selected", set is {a, b}.
//   - `ns=*`               -> "full": pass-through, no filtering.
//
// The code-mode surface is assumed throughout; the flat surface is out
// of scope for v1 (the router strips any caller `codemode` parameter).

export const DEFAULT_FILTER_PROFILE = "conservative";

// Tools a client may see and call in each non-full profile. The
// selected profile adds exactly the two dispatch tools; exec and
// tool_invoke (and every other mcpx_* extra) are never exposed here.
export const DISCOVERY_TOOLS = [
  "mcpx_namespaces",
  "mcpx_catalog",
  "mcpx_types",
  "mcpx_search",
] as const;
export const SELECTED_DISPATCH_TOOLS = ["mcpx_call", "mcpx_batch"] as const;

export type FilterMode = "conservative" | "selected" | "full";

export interface FilterSelector {
  // The raw `ns` value as presented (trimmed), or null when absent.
  raw: string | null;
  // The effective profile name: the raw selector when one was given,
  // otherwise DEFAULT_FILTER_PROFILE.
  profile: string;
  // The enforcement mode derived from the raw value.
  mode: FilterMode;
  // The selected namespace set (empty for conservative and full).
  namespaces: string[];
}

export interface JsonRpcMessage {
  jsonrpc?: string;
  id?: unknown;
  method?: string;
  params?: Record<string, unknown>;
  result?: unknown;
  error?: unknown;
}

export function resolveFilterSelector(url: URL): FilterSelector {
  const raw = url.searchParams.get("ns")?.trim() || null;
  if (raw === "*") {
    return { raw, profile: raw, mode: "full", namespaces: [] };
  }
  if (raw !== null) {
    const namespaces = raw
      .split(",")
      .map((entry) => entry.trim())
      .filter((entry) => entry.length > 0);
    // `ns=` (or only separators) selects nothing: keep it "selected"
    // so the effective set is still enforced as empty rather than
    // silently widening to the conservative discovery set.
    return { raw, profile: raw, mode: "selected", namespaces };
  }
  return {
    raw: null,
    profile: DEFAULT_FILTER_PROFILE,
    mode: "conservative",
    namespaces: [],
  };
}

function exposedToolNames(mode: FilterMode): readonly string[] {
  if (mode === "full") return [];
  if (mode === "selected") return [...DISCOVERY_TOOLS, ...SELECTED_DISPATCH_TOOLS];
  return DISCOVERY_TOOLS;
}

function namespacePart(name: string): string {
  const dot = name.indexOf(".");
  return dot >= 0 ? name.slice(0, dot) : name;
}

function inSelectedSet(selector: FilterSelector, namespace: string): boolean {
  return selector.namespaces.includes(namespace);
}

// mcpx's own resolution rules: an empty tool with a dotted namespace
// splits on the first dot; an empty namespace with a dotted tool does
// the same the other way round.
function resolveTarget(
  namespace: unknown,
  tool: unknown,
): { namespace: string; tool: string } {
  let ns = typeof namespace === "string" ? namespace.trim() : "";
  let name = typeof tool === "string" ? tool.trim() : "";
  if (!name && ns.includes(".")) {
    const dot = ns.indexOf(".");
    name = ns.slice(dot + 1);
    ns = ns.slice(0, dot);
  } else if (!ns && name.includes(".")) {
    const dot = name.indexOf(".");
    ns = name.slice(0, dot);
    name = name.slice(dot + 1);
  }
  return { namespace: ns, tool: name };
}

function isGatewayTool(toolName: string): boolean {
  // Every mcpx_* tool is a door/gateway tool (exec, tool_invoke,
  // mcpx_batch, and any future extra); batch_call is the alias.
  return toolName.startsWith("mcpx_") || toolName === "batch_call";
}

function stringList(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

function gatewayCallAllowed(selector: FilterSelector, message: JsonRpcMessage): boolean {
  const params = message.params ?? {};
  const name = typeof params.name === "string" ? params.name : "";
  const args = (params.arguments ?? {}) as Record<string, unknown>;

  if (selector.mode === "full") return true;

  switch (name) {
    case "mcpx_namespaces":
    case "mcpx_catalog":
    case "mcpx_search":
      return true;
    case "mcpx_types": {
      if (selector.mode !== "selected") return false;
      const requested = stringList(args.namespaces);
      // With no namespace list the upstream would list every
      // namespace's types, which leaks out-of-set names: reject.
      if (requested.length === 0) return false;
      return requested.every((entry) => inSelectedSet(selector, namespacePart(entry)));
    }
    case "mcpx_call": {
      if (selector.mode !== "selected") return false;
      const target = resolveTarget(args.namespace, args.tool);
      if (!target.namespace) return false;
      return inSelectedSet(selector, target.namespace);
    }
    case "mcpx_batch":
    case "batch_call": {
      if (selector.mode !== "selected") return false;
      const calls = Array.isArray(args.calls) ? args.calls : [];
      if (calls.length === 0) return false;
      return calls.every((raw) => {
        if (!raw || typeof raw !== "object") return false;
        const call = raw as Record<string, unknown>;
        const target = resolveTarget(
          call.namespace,
          call.tool ?? call.name,
        );
        if (!target.namespace || !target.tool) return false;
        if (isGatewayTool(target.tool) || isGatewayTool(target.namespace)) return false;
        return inSelectedSet(selector, target.namespace);
      });
    }
    default:
      return false;
  }
}

function uriAllowed(selector: FilterSelector, uri: unknown): boolean {
  if (selector.mode === "full") return true;
  if (typeof uri !== "string" || !uri.startsWith("mcpx://")) return true;
  const rest = uri.slice("mcpx://".length);
  const slash = rest.indexOf("/");
  const namespace = slash >= 0 ? rest.slice(0, slash) : rest;
  return inSelectedSet(selector, namespace);
}

// True when the request may be sent upstream.
export function mcpRequestAllowed(
  message: JsonRpcMessage,
  selector: FilterSelector,
): boolean {
  if (selector.mode === "full") return true;
  if (!message || typeof message !== "object") return true;

  if (message.method === "tools/call") {
    return gatewayCallAllowed(selector, message);
  }
  if (message.method === "prompts/get" || message.method === "resources/read") {
    const params = message.params ?? {};
    return uriAllowed(selector, params.uri);
  }
  // initialize, ping, notifications/*, tools/list, prompts/list,
  // resources/list and anything else pass through; list responses are
  // filtered on the way back.
  return true;
}

export const NOT_EXPOSED_ERROR = {
  code: -32602,
  message: "not exposed by this door's selector",
} as const;

export function jsonRpcError(id: unknown): JsonRpcMessage {
  return { jsonrpc: "2.0", id: id ?? null, error: NOT_EXPOSED_ERROR };
}

export interface GatedRequest {
  // The body to send upstream (may be empty when every element was
  // rejected, in which case no upstream call is needed).
  bodyText: string;
  // The messages that will reach upstream, in request order.
  allowed: JsonRpcMessage[];
  // Error objects to return for rejected elements.
  rejected: JsonRpcMessage[];
  // True when the payload is a JSON-RPC batch array.
  batch: boolean;
}

// Gate a raw request body. Returns null when the body is not JSON-RPC
// we can police (the caller then forwards it untouched).
export function gateMcpRequestBody(
  raw: string,
  selector: FilterSelector,
): GatedRequest | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(trimmed);
  } catch {
    return null;
  }

  const elements = Array.isArray(parsed) ? parsed : [parsed];
  if (!elements.some((el) => el && typeof el === "object" && "method" in (el as object))) {
    return null;
  }

  const allowed: JsonRpcMessage[] = [];
  const rejected: JsonRpcMessage[] = [];
  for (const element of elements) {
    const message = element as JsonRpcMessage;
    if (mcpRequestAllowed(message, selector)) allowed.push(message);
    else rejected.push(jsonRpcError(message.id));
  }

  if (rejected.length === 0) {
    return { bodyText: raw, allowed, rejected, batch: Array.isArray(parsed) };
  }
  const bodyText = Array.isArray(parsed)
    ? JSON.stringify(allowed)
    : allowed.length > 0
      ? JSON.stringify(allowed[0])
      : "";
  return { bodyText, allowed, rejected, batch: Array.isArray(parsed) };
}

// ------------------------------------------------------------- responses

function filterToolList(result: unknown, selector: FilterSelector): unknown {
  if (!result || typeof result !== "object") return result;
  const record = result as Record<string, unknown>;
  if (!Array.isArray(record.tools)) return result;
  const exposed = exposedToolNames(selector.mode);
  return {
    ...record,
    tools: (record.tools as Array<Record<string, unknown>>).filter(
      (tool) => typeof tool?.name === "string" && exposed.includes(tool.name),
    ),
  };
}

function filterNamespaceValue(value: unknown, selector: FilterSelector): unknown {
  if (typeof value === "string") {
    return inSelectedSet(selector, namespacePart(value)) ? value : undefined;
  }
  if (Array.isArray(value)) {
    return value
      .map((entry) => filterNamespaceValue(entry, selector))
      .filter((entry) => entry !== undefined);
  }
  if (value && typeof value === "object") {
    const record = value as Record<string, unknown>;
    const out: Record<string, unknown> = {};
    for (const [key, inner] of Object.entries(record)) {
      if (key === "namespaces" || key === "results" || key === "items") {
        const filtered = filterNamespaceValue(inner, selector);
        out[key] = filtered === undefined ? [] : filtered;
      } else {
        out[key] = inner;
      }
    }
    // A bare namespace object: expose it only when its own name is in
    // the selected set.
    const name = typeof record.name === "string" ? record.name : undefined;
    const namespace =
      typeof record.namespace === "string" ? record.namespace : undefined;
    const identity = name ?? namespace;
    if (identity !== undefined && !inSelectedSet(selector, namespacePart(identity))) {
      return undefined;
    }
    return out;
  }
  return value;
}

function filterNamespaceResult(result: unknown, selector: FilterSelector): unknown {
  if (!result || typeof result !== "object") return result;
  const record = result as Record<string, unknown>;
  const out: Record<string, unknown> = { ...record };

  if ("structuredContent" in out) {
    const filtered = filterNamespaceValue(out.structuredContent, selector);
    out.structuredContent = filtered === undefined ? [] : filtered;
  }
  if (Array.isArray(out.content)) {
    out.content = (out.content as Array<Record<string, unknown>>).map((entry) => {
      if (entry?.type !== "text" || typeof entry.text !== "string") return entry;
      try {
        const parsed = JSON.parse(entry.text);
        const filtered = filterNamespaceValue(parsed, selector);
        if (filtered === undefined) return { ...entry, text: "[]" };
        return { ...entry, text: JSON.stringify(filtered) };
      } catch {
        return entry;
      }
    });
  }
  return out;
}

function filterUriList(result: unknown, selector: FilterSelector): unknown {
  if (!result || typeof result !== "object") return result;
  const record = result as Record<string, unknown>;
  const key = Array.isArray(record.resources)
    ? "resources"
    : Array.isArray(record.prompts)
      ? "prompts"
      : null;
  if (!key) return result;
  return {
    ...record,
    [key]: (record[key] as Array<Record<string, unknown>>).filter((entry) =>
      uriAllowed(selector, entry?.uri),
    ),
  };
}

function transformResponseMessage(
  message: JsonRpcMessage,
  selector: FilterSelector,
  requests: JsonRpcMessage[],
): JsonRpcMessage {
  if (!message || typeof message !== "object" || !("result" in message) || "method" in message) {
    return message;
  }
  const request = requests.find((candidate) => candidate.id === message.id);
  if (!request) return message;

  if (request.method === "tools/list") {
    return { ...message, result: filterToolList(message.result, selector) };
  }
  if (request.method === "tools/call") {
    const name = request.params?.name;
    if (name === "mcpx_namespaces") {
      return { ...message, result: filterNamespaceResult(message.result, selector) };
    }
    return message;
  }
  if (request.method === "prompts/list" || request.method === "resources/list") {
    return { ...message, result: filterUriList(message.result, selector) };
  }
  return message;
}

function transformBody(
  body: string,
  selector: FilterSelector,
  requests: JsonRpcMessage[],
  isSse: boolean,
): string {
  if (isSse) {
    return body
      .split("\n")
      .map((line) => {
        if (!line.startsWith("data:")) return line;
        const payload = line.slice("data:".length).trim();
        if (!payload || payload === "[DONE]") return line;
        try {
          const parsed = JSON.parse(payload) as JsonRpcMessage | JsonRpcMessage[];
          const transformed = Array.isArray(parsed)
            ? parsed.map((message) => transformResponseMessage(message, selector, requests))
            : transformResponseMessage(parsed, selector, requests);
          return `data: ${JSON.stringify(transformed)}`;
        } catch {
          return line;
        }
      })
      .join("\n");
  }
  try {
    const parsed = JSON.parse(body) as JsonRpcMessage | JsonRpcMessage[];
    const transformed = Array.isArray(parsed)
      ? parsed.map((message) => transformResponseMessage(message, selector, requests))
      : transformResponseMessage(parsed, selector, requests);
    return JSON.stringify(transformed);
  } catch {
    return body;
  }
}

// Filter an MCP response coming back from the bridge. `full` passes the
// original (streaming) response through untouched; the other profiles
// buffer, transform and re-emit in the same framing.
export async function filterMcpResponse(
  response: Response,
  selector: FilterSelector,
  requests: JsonRpcMessage[] = [],
): Promise<Response> {
  if (selector.mode === "full") return response;

  const contentType = response.headers.get("content-type") ?? "";
  const isSse = contentType.toLowerCase().includes("text/event-stream");
  const text = await response.text();
  const headers = new Headers(response.headers);
  headers.delete("content-length");
  const transformed = transformBody(text, selector, requests, isSse);
  return new Response(transformed, {
    status: response.status,
    statusText: response.statusText,
    headers,
  });
}

// Merge the upstream response to an allowed subset of a batch with the
// error objects for the rejected elements.
export async function mergeBatchRejections(
  response: Response,
  selector: FilterSelector,
  requests: JsonRpcMessage[],
  rejected: JsonRpcMessage[],
): Promise<Response> {
  const filtered = await filterMcpResponse(response, selector, requests);
  const contentType = (filtered.headers.get("content-type") ?? "").toLowerCase();
  const headers = new Headers(filtered.headers);
  headers.delete("content-length");

  if (contentType.includes("text/event-stream")) {
    const body = await filtered.text();
    const events = rejected
      .map((error) => `event: message\ndata: ${JSON.stringify(error)}\n\n`)
      .join("");
    return new Response(body + events, {
      status: filtered.status,
      statusText: filtered.statusText,
      headers,
    });
  }

  const body = await filtered.text();
  let parsed: unknown = [];
  if (body.trim()) {
    try {
      parsed = JSON.parse(body);
    } catch {
      parsed = [];
    }
  }
  const combined = Array.isArray(parsed) ? [...parsed, ...rejected] : [parsed, ...rejected];
  headers.set("content-type", "application/json");
  return new Response(JSON.stringify(combined), {
    status: filtered.status,
    statusText: filtered.statusText,
    headers,
  });
}
