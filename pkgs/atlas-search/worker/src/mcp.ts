/**
 * MCP (Model Context Protocol) endpoint (#8625): the hosted
 * Atlas query surface as a stateless Streamable HTTP MCP
 * server, for mcpx and any other MCP client.
 *
 * One tool — `search` — over the exact same execution path as
 * POST /v1/search (src/search.ts): the gate and the agent's
 * policy ceiling resolve in index.ts before dispatch, and the
 * search itself carries that ceiling and the pinned
 * `hosted-per-user` disclosure context with the same per-query
 * Durable Object cache. Callers authenticated by AGENT_TOKENS
 * each search under their own ceiling; callers on the shared
 * gate token search under the surface pin. Corpus scoping is the
 * named follow-on slice (no corpus dimension exists in the
 * branch queries yet) and is deliberately not improvised here.
 *
 * Nothing else is exposed: cache purge is a write and cache
 * inspect is an operator/audit surface, so neither becomes a
 * tool.
 *
 * Stateless by design: no session id is issued or required.
 * Each request is one JSON-RPC message (or a batch of them)
 * answered with a plain JSON response; notifications are
 * accepted and answered 202 with no body.
 */
import { readTextUpTo } from "./body";
import { normalizeQuery } from "./key";
import { clampK, executeSearch, type SearchEnv } from "./search";
import type { Ceiling } from "./space";

const SUPPORTED_VERSIONS = ["2024-11-05", "2025-03-26", "2025-06-18"];
const FALLBACK_VERSION = "2025-03-26";
const SERVER_INFO = { name: "atlas-search", version: "0.1.0" };

const SEARCH_TOOL = {
  name: "search",
  description:
    "Search the unified Atlas index — all of Drew's Atlas databases — for a natural-language query. " +
    "Runs under your agent ceiling (at most personal) in the hosted-per-user disclosure context. " +
    "Returns the search envelope: cache status, the result list with per-hit provenance, " +
    "and the effective ceiling the service enforced.",
  inputSchema: {
    type: "object",
    properties: {
      query: { type: "string", description: "The query text." },
      k: {
        type: "integer",
        minimum: 1,
        maximum: 50,
        default: 10,
        description: "Maximum number of results to return.",
      },
    },
    required: ["query"],
  },
  annotations: { readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false },
};

// MCP 2025-06-18 introduced structured tool output declarations. The
// result and metadata objects remain open so service extensions and cached
// results from earlier generations share the same envelope contract.
const STRUCTURED_SEARCH_TOOL = {
  ...SEARCH_TOOL,
  outputSchema: {
    type: "object",
    properties: {
      cache: { type: "string", enum: ["hit", "miss"] },
      key: { type: "string" },
      query: { type: "string" },
      result: { type: "object" },
      meta: { type: "object" },
    },
    required: ["cache", "key", "query", "result", "meta"],
  },
};

type JsonRpcId = string | number | null;

interface JsonRpcMessage {
  id?: JsonRpcId;
  method?: unknown;
  params?: unknown;
}

function rpcResult(id: JsonRpcId, result: unknown): Record<string, unknown> {
  return { jsonrpc: "2.0", id, result };
}

function rpcError(id: JsonRpcId, code: number, message: string): Record<string, unknown> {
  return { jsonrpc: "2.0", id, error: { code, message } };
}

function paramsObject(msg: JsonRpcMessage): Record<string, unknown> {
  return typeof msg.params === "object" && msg.params !== null ? (msg.params as Record<string, unknown>) : {};
}

async function recordQueryAudit(
  agentId: string,
  ceiling: Ceiling,
  query: string,
  k: number,
  status: number,
  cache?: string,
): Promise<void> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(query));
  const queryHash = Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
  // Worker logs are the audit sink. Do not retain raw query text: its
  // stable digest is enough to correlate an execution without exposing it.
  console.log(
    JSON.stringify({
      event: "atlas_mcp_query",
      agent_id: agentId,
      ceiling,
      query_sha256: queryHash,
      k,
      status,
      ...(cache === undefined ? {} : { cache }),
      at: new Date().toISOString(),
    }),
  );
}

/**
 * Handle one JSON-RPC message. Returns the response object, or
 * null for a notification (a message with no `id` is never
 * executed and never answered — including a tools/call sent
 * as a notification).
 */
async function handleMessage(
  msg: JsonRpcMessage,
  env: SearchEnv,
  protocolVersion: string | null,
  agentId: string,
  ceiling: Ceiling,
): Promise<Record<string, unknown> | null> {
  if (!("id" in msg)) return null;
  const id: JsonRpcId = msg.id ?? null;
  if (typeof msg.method !== "string") {
    return rpcError(id, -32600, "invalid request: method must be a string");
  }
  switch (msg.method) {
    case "initialize": {
      const requested = paramsObject(msg).protocolVersion;
      const protocolVersion =
        typeof requested === "string" && SUPPORTED_VERSIONS.includes(requested) ? requested : FALLBACK_VERSION;
      return rpcResult(id, {
        protocolVersion,
        capabilities: { tools: {} },
        serverInfo: SERVER_INFO,
        instructions:
          "Atlas search: one tool, `search`, over Drew's unified Atlas index under your agent ceiling (at most personal). Results are cached per query and ceiling; a repeat query is served from cache.",
      });
    }
    case "ping":
      return rpcResult(id, {});
    case "tools/list":
      return rpcResult(id, { tools: [protocolVersion === "2025-06-18" ? STRUCTURED_SEARCH_TOOL : SEARCH_TOOL] });
    case "tools/call": {
      const params = paramsObject(msg);
      if (params.name !== SEARCH_TOOL.name) {
        return rpcError(id, -32602, `unknown tool: ${String(params.name)}`);
      }
      const args =
        typeof params.arguments === "object" && params.arguments !== null
          ? (params.arguments as Record<string, unknown>)
          : {};
      if (typeof args.query !== "string") {
        return rpcError(id, -32602, "arguments.query must be a string");
      }
      const query = normalizeQuery(args.query);
      if (!query) return rpcError(id, -32602, "arguments.query must not be empty");
      const k = clampK(args.k);
      let outcome: Awaited<ReturnType<typeof executeSearch>>;
      try {
        outcome = await executeSearch(env, query, k, { ceiling });
      } catch (error) {
        await recordQueryAudit(agentId, ceiling, query, k, 500);
        throw error;
      }
      const { status, payload } = outcome;
      await recordQueryAudit(
        agentId,
        ceiling,
        query,
        k,
        status,
        typeof payload.cache === "string" ? payload.cache : undefined,
      );
      const text = JSON.stringify(payload);
      if (status !== 200) {
        return rpcResult(id, { content: [{ type: "text", text }], isError: true });
      }
      return rpcResult(id, { content: [{ type: "text", text }], structuredContent: payload, isError: false });
    }
    default:
      return rpcError(id, -32601, `method not found: ${msg.method}`);
  }
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
}

/**
 * The POST /mcp handler. The caller (index.ts) has already
 * applied the gate and resolved the caller's policy ceiling;
 * this owns the JSON-RPC layer only.
 */
export async function handleMcp(req: Request, env: SearchEnv, agentId: string, ceiling: Ceiling): Promise<Response> {
  const raw = await readTextUpTo(req);
  if (raw === null) {
    return jsonResponse(413, rpcError(null, -32600, "body is larger than the 64 KiB limit"));
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return jsonResponse(400, rpcError(null, -32700, "parse error: body must be JSON"));
  }
  const protocolVersion = req.headers.get("MCP-Protocol-Version");
  if (Array.isArray(parsed)) {
    if (parsed.length === 0) {
      return jsonResponse(400, rpcError(null, -32600, "invalid request: empty batch"));
    }
    const responses: Record<string, unknown>[] = [];
    for (const item of parsed) {
      if (typeof item !== "object" || item === null) {
        responses.push(rpcError(null, -32600, "invalid request: batch items must be objects"));
        continue;
      }
      const resp = await handleMessage(item as JsonRpcMessage, env, protocolVersion, agentId, ceiling);
      if (resp !== null) responses.push(resp);
    }
    if (responses.length === 0) return new Response(null, { status: 202 });
    return jsonResponse(200, responses);
  }
  if (typeof parsed !== "object" || parsed === null) {
    return jsonResponse(400, rpcError(null, -32600, "invalid request: body must be a JSON object or batch"));
  }
  const resp = await handleMessage(parsed as JsonRpcMessage, env, protocolVersion, agentId, ceiling);
  if (resp === null) return new Response(null, { status: 202 });
  return jsonResponse(200, resp);
}
