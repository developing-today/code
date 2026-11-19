// ChatGPT connector: the private Site's Worker.
//
// It keeps the Site's own address and sign-in: Sites authenticates every
// request and supplies oai-authenticated-user-id, and nothing reaches
// the tools without it. The tools themselves are the message bridge's
// (pkgs/message-bridge): this Worker passes each MCP request to the
// bridge's /mcp on the ChatGPT channel, so ChatGPT and Claude run the
// same send_message / read_messages / ack_messages code over one store,
// and Drew's page can send to either or both.
//
// Configuration (none of it in git):
//   MESSAGE_BRIDGE_TOKEN  the bridge's CHATGPT_TOKEN (Bitwarden
//                         message-bridge.chatgpt-token). A Site secret
//                         when the Site has secrets; otherwise the R2
//                         object config/message-bridge-token in BUCKET.
//                         It opens only the ChatGPT channel's /mcp.
//   MESSAGE_BRIDGE_URL    optional; defaults to https://bridge.security.cab.
//   OWNER_USER_ID         optional; when set, only that Sites user id.
// The token never reaches the model, the caller or a response.

const DEFAULT_BRIDGE = "https://bridge.security.cab";
const TOKEN_KEY = "config/message-bridge-token";
const MAX_REQUEST_BYTES = 64 * 1024;
const MIN_TOKEN_LENGTH = 32;
const headers = { "Cache-Control": "no-store" };

const rpcError = (id, code, message, status = 200) =>
  Response.json({ jsonrpc: "2.0", id: id ?? null, error: { code, message } }, { status, headers });

async function bridgeToken(env) {
  let token = typeof env.MESSAGE_BRIDGE_TOKEN === "string" ? env.MESSAGE_BRIDGE_TOKEN.trim() : "";
  if (!token && env.BUCKET) {
    try {
      token = ((await (await env.BUCKET.get(TOKEN_KEY))?.text()) ?? "").trim();
    } catch {
      token = "";
    }
  }
  return token.length >= MIN_TOKEN_LENGTH && /^[A-Za-z0-9_-]+$/.test(token) ? token : null;
}

function bridgeUrl(env) {
  const raw = typeof env.MESSAGE_BRIDGE_URL === "string" && env.MESSAGE_BRIDGE_URL ? env.MESSAGE_BRIDGE_URL : DEFAULT_BRIDGE;
  const url = new URL(raw);
  if (url.protocol !== "https:") throw new Error("MESSAGE_BRIDGE_URL must be https");
  return url.origin;
}

export default {
  async fetch(request, env) {
    const path = new URL(request.url).pathname;
    if (path === "/" && request.method === "GET") {
      return new Response(
        "Private ChatGPT message connector. Messages sent here land in Drew's message bridge, next to Claude's.",
        { headers: { ...headers, "Content-Type": "text/plain; charset=utf-8" } },
      );
    }
    if (path !== "/mcp") return new Response(null, { status: 404, headers });
    if (request.method !== "POST") return new Response(null, { status: 405, headers: { ...headers, Allow: "POST" } });
    // Trust this header only on Site-hosted ingress where Sites authenticates and supplies it.
    const user = request.headers.get("oai-authenticated-user-id");
    if (!user || user.length > 300) return new Response(null, { status: 401, headers });
    if (env.OWNER_USER_ID && user !== env.OWNER_USER_ID) return new Response(null, { status: 403, headers });

    const body = await request.text();
    if (body.length > MAX_REQUEST_BYTES) return new Response(null, { status: 413, headers });
    let data;
    try {
      data = JSON.parse(body);
    } catch {
      return rpcError(null, -32700, "Parse error", 400);
    }
    const id = Array.isArray(data) ? null : data?.id;

    const token = await bridgeToken(env);
    if (!token) return rpcError(id, -32603, "The message bridge isn't configured for this connector yet.");
    let res;
    try {
      // Only the JSON-RPC body goes on: no caller headers, no user id.
      res = await fetch(`${bridgeUrl(env)}/mcp/${token}`, {
        method: "POST",
        headers: { "content-type": "application/json", accept: "application/json, text/event-stream" },
        body,
        redirect: "manual",
      });
    } catch {
      return rpcError(id, -32603, "The message bridge couldn't be reached.");
    }
    if (res.status === 404) return rpcError(id, -32603, "The message bridge didn't accept this connector's token.");
    if (res.status === 202) return new Response(null, { status: 202, headers });
    const type = res.headers.get("content-type") ?? "";
    if (!type.includes("application/json")) {
      return rpcError(id, -32603, `The message bridge answered ${res.status}.`);
    }
    return new Response(await res.text(), {
      status: res.status,
      headers: { ...headers, "Content-Type": "application/json" },
    });
  },
};
