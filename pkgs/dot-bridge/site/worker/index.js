const tools = [
  {name: "write_probe", description: "Store only a fixed synthetic probe value for the authenticated caller.", inputSchema: {type: "object", properties: {}, additionalProperties: false}, annotations: {readOnlyHint: false, destructiveHint: false, idempotentHint: true}},
  {name: "read_probe", description: "Check whether the authenticated caller's synthetic probe was stored.", inputSchema: {type: "object", properties: {}, additionalProperties: false}, annotations: {readOnlyHint: true, idempotentHint: true}},
];
const headers = {"Cache-Control": "no-store"};
export default { async fetch(request, env) {
  if (new URL(request.url).pathname === "/" && request.method === "GET") return new Response("Private Aster Mailbox Connection Test. This plugin stores only a fixed synthetic test marker.", {headers: {...headers,"Content-Type":"text/plain; charset=utf-8"}});
  if (new URL(request.url).pathname !== "/mcp") return new Response(null, {status: 404, headers});
  if (request.method !== "POST") return new Response(null, {status: 405, headers});
  // Trust this header only on Site-hosted ingress where Sites authenticates and supplies it.
  const user = request.headers.get("oai-authenticated-user-id");
  if (!user || user.length > 300) return new Response(null, {status: 401, headers});
  let data;
  try { const body = await request.text(); if (body.length > 4096) return new Response(null, {status: 413, headers}); data = JSON.parse(body); }
  catch { return new Response(null, {status: 400, headers}); }
  const id = data?.id ?? null;
  const wrap = (key, value) => Response.json({jsonrpc: "2.0", id, [key]: value}, {headers});
  const result = value => wrap("result", value);
  const error = (code, message) => wrap("error", {code,message});
  if (data?.jsonrpc !== "2.0" || typeof data?.method !== "string") return error(-32600, "Invalid request");
  if (data.method === "server/discover") return result({supportedVersions: ["2026-07-28"], capabilities: {tools: {}}});
  if (data.method === "tools/list") return result({tools});
  if (data.method !== "tools/call") return error(-32601, "Unknown method");
  const {name, arguments: args = {}} = data.params ?? {};
  const output = (text, isError = false) => result({content: [{type:"text",text}], isError});
  if (!args || typeof args !== "object" || Array.isArray(args) || Object.keys(args).length) return output("The probe accepts no data.", true);
  try {
    if (name === "write_probe") {
      await env.BUCKET.put("probe/" + encodeURIComponent(user), "site-probe-v1");
      return output("Synthetic probe stored.");
    }
    if (name === "read_probe") {
      const row = await env.BUCKET.head("probe/" + encodeURIComponent(user));
      return output(row ? "site-probe-v1" : "No probe stored.");
    }
    return output("Unknown tool.", true);
  } catch { return output("Probe storage unavailable.", true); }
}};
