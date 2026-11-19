// Minimal remote MCP server over Streamable HTTP, stateless.
//
// Each POST carries one JSON-RPC message (or a batch) and gets a plain
// application/json answer; notifications get 202 with no body. There is
// no server-initiated stream, so GET and DELETE answer 405, which the
// spec allows. No session id is issued: every call stands alone, and
// the message store is the only state.

import { InputError, MAX_BODY, MAX_LIMIT } from "./messages.js";

export const SERVER_NAME = "message-bridge";
export const SERVER_VERSION = "0.1.0";
export const PROTOCOL_VERSIONS = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const MAX_REQUEST_BYTES = 64 * 1024;

export const INSTRUCTIONS =
  "A message bridge between Claude and Drew. send_message stores a message from Claude (outbound). " +
  "read_messages returns stored messages; inbound ones were sent to Claude from Drew's page or the JSON API. " +
  "Use read_messages with unread_only to check for new inbound messages, then ack_messages (or mark_read) " +
  "so they are not reported as new again. Every message is kept with its direction, source and time.";

export const TOOLS = [
  {
    name: "send_message",
    title: "Send a message",
    description:
      "Send (push) a message from Claude. It is stored as an outbound message and shows in the history page. Returns the stored message.",
    inputSchema: {
      type: "object",
      properties: {
        body: { type: "string", description: "Message text.", maxLength: MAX_BODY },
        sender: { type: "string", description: "Optional label for who is sending, e.g. 'claude'." },
      },
      required: ["body"],
      additionalProperties: false,
    },
    annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: false, openWorldHint: false },
  },
  {
    name: "read_messages",
    title: "Read messages",
    description:
      "Read (receive) stored messages, oldest first. Defaults to inbound messages (sent to Claude). " +
      "Use since_id with the returned next_since_id to page or to poll for new ones; unread_only returns inbound messages not yet acknowledged; mark_read acknowledges the inbound ones returned.",
    inputSchema: {
      type: "object",
      properties: {
        direction: { type: "string", enum: ["inbound", "outbound", "all"], default: "inbound" },
        since_id: { type: "integer", minimum: 0, description: "Only messages with an id above this." },
        unread_only: { type: "boolean", default: false },
        limit: { type: "integer", minimum: 1, maximum: MAX_LIMIT, default: 50 },
        mark_read: { type: "boolean", default: false, description: "Acknowledge the inbound messages returned." },
      },
      additionalProperties: false,
    },
    annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: true, openWorldHint: false },
  },
  {
    name: "ack_messages",
    title: "Acknowledge messages",
    description: "Mark inbound messages as read, by id list or everything up to an id.",
    inputSchema: {
      type: "object",
      properties: {
        up_to_id: { type: "integer", minimum: 0 },
        ids: { type: "array", items: { type: "integer", minimum: 1 }, maxItems: 1000 },
      },
      additionalProperties: false,
    },
    annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: true, openWorldHint: false },
  },
];

const rpcError = (id, code, message) => ({ jsonrpc: "2.0", id: id ?? null, error: { code, message } });
const rpcResult = (id, result) => ({ jsonrpc: "2.0", id, result });

function toolResult(data, isError = false) {
  return {
    content: [{ type: "text", text: JSON.stringify(data, null, 2) }],
    ...(isError ? { isError: true } : { structuredContent: data }),
  };
}

function rejectUnknownArgs(args, allowed) {
  for (const key of Object.keys(args)) {
    if (!allowed.includes(key)) throw new InputError(`unknown argument: ${key}`);
  }
}

export async function callTool(name, args, store) {
  if (args === undefined || args === null) args = {};
  if (typeof args !== "object" || Array.isArray(args)) throw new InputError("arguments must be an object");
  switch (name) {
    case "send_message": {
      rejectUnknownArgs(args, ["body", "sender"]);
      const message = await store.add({ body: args.body, sender: args.sender, direction: "outbound", source: "mcp" });
      return { message };
    }
    case "read_messages": {
      rejectUnknownArgs(args, ["direction", "since_id", "unread_only", "limit", "mark_read"]);
      const direction = args.direction ?? "inbound";
      const { messages } = await store.list({
        direction,
        since_id: args.since_id,
        unread_only: args.unread_only,
        limit: args.limit,
      });
      let acked = 0;
      if (args.mark_read) {
        const ids = messages.filter((m) => m.direction === "inbound" && m.read_at === null).map((m) => m.id);
        if (ids.length) acked = (await store.ack({ ids })).acked;
      }
      const stats = await store.stats();
      const nextSinceId = messages.length ? messages[messages.length - 1].id : (args.since_id ?? 0);
      return { messages, next_since_id: nextSinceId, ...(args.mark_read ? { acked } : {}), stats };
    }
    case "ack_messages": {
      rejectUnknownArgs(args, ["up_to_id", "ids"]);
      return await store.ack({ up_to_id: args.up_to_id, ids: args.ids });
    }
    default:
      return null;
  }
}

async function handleOne(msg, store) {
  if (!msg || typeof msg !== "object" || msg.jsonrpc !== "2.0" || typeof msg.method !== "string") {
    return rpcError(msg?.id, -32600, "Invalid Request");
  }
  const isNotification = !("id" in msg);
  const { id, method, params } = msg;
  if (isNotification) return null;
  switch (method) {
    case "initialize": {
      const asked = params?.protocolVersion;
      const protocolVersion = PROTOCOL_VERSIONS.includes(asked) ? asked : PROTOCOL_VERSIONS[0];
      return rpcResult(id, {
        protocolVersion,
        capabilities: { tools: { listChanged: false } },
        serverInfo: { name: SERVER_NAME, title: "Message bridge", version: SERVER_VERSION },
        instructions: INSTRUCTIONS,
      });
    }
    case "ping":
      return rpcResult(id, {});
    case "tools/list":
      return rpcResult(id, { tools: TOOLS });
    case "tools/call": {
      const name = params?.name;
      if (!TOOLS.some((t) => t.name === name)) return rpcError(id, -32602, `Unknown tool: ${name}`);
      try {
        return rpcResult(id, toolResult(await callTool(name, params?.arguments, store)));
      } catch (err) {
        if (err instanceof InputError) return rpcResult(id, toolResult({ error: err.message }, true));
        throw err;
      }
    }
    case "resources/list":
      return rpcResult(id, { resources: [] });
    case "prompts/list":
      return rpcResult(id, { prompts: [] });
    default:
      return rpcError(id, -32601, `Method not found: ${method}`);
  }
}

const json = (data, status = 200) =>
  new Response(JSON.stringify(data), {
    status,
    headers: { "content-type": "application/json", "cache-control": "no-store" },
  });

/** Handles a request already authorized for /mcp/<token>. */
export async function handleMcp(request, store) {
  if (request.method !== "POST") {
    return new Response("Method Not Allowed", { status: 405, headers: { allow: "POST" } });
  }
  const text = await request.text();
  if (text.length > MAX_REQUEST_BYTES) return json(rpcError(null, -32600, "Request too large"), 413);
  let payload;
  try {
    payload = JSON.parse(text);
  } catch {
    return json(rpcError(null, -32700, "Parse error"), 400);
  }
  if (Array.isArray(payload)) {
    if (payload.length === 0) return json(rpcError(null, -32600, "Invalid Request"), 400);
    const out = [];
    for (const m of payload) {
      const r = await handleOne(m, store);
      if (r) out.push(r);
    }
    return out.length ? json(out) : new Response(null, { status: 202 });
  }
  const r = await handleOne(payload, store);
  return r ? json(r) : new Response(null, { status: 202 });
}
