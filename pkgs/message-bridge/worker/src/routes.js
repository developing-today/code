// Request router. Pure: takes the request, env and a message store, so
// the tests can drive it with MemoryStore and index.js with the
// Durable Object stub.
//
//   POST /mcp/<token>                 remote MCP server (Streamable HTTP)
//   GET  /p/<token>                   history page with the send form
//   POST /p/<token>                   form post: store an inbound message
//   GET  /api/<token>/messages        JSON history (direction, since_id,
//                                     unread_only, limit, order)
//   POST /api/<token>/messages        JSON {body, sender}: store inbound
//   POST /api/<token>/ack             JSON {up_to_id | ids}: mark read
//   GET  /healthz                     {"ok":true}, no data
// Anything else, a wrong token included, is a plain 404.

import { matchTokenPath, tokenConfigured } from "./auth.js";
import { handleMcp } from "./mcp.js";
import { InputError } from "./messages.js";
import { renderPage } from "./page.js";

const MAX_FORM_BYTES = 64 * 1024;

const PRIVATE_HEADERS = {
  "cache-control": "no-store",
  "referrer-policy": "no-referrer",
  "x-robots-tag": "noindex, nofollow",
  "x-content-type-options": "nosniff",
};

const PAGE_CSP =
  "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; " +
  "form-action 'self'; frame-ancestors 'none'; base-uri 'none'";

function withHeaders(res, extra = {}) {
  const h = new Headers(res.headers);
  for (const [k, v] of Object.entries({ ...PRIVATE_HEADERS, ...extra })) h.set(k, v);
  return new Response(res.body, { status: res.status, statusText: res.statusText, headers: h });
}

const json = (data, status = 200) =>
  new Response(JSON.stringify(data), { status, headers: { "content-type": "application/json" } });

const notFound = () => new Response("Not found\n", { status: 404, headers: { "content-type": "text/plain" } });

async function readBody(request) {
  const text = await request.text();
  if (text.length > MAX_FORM_BYTES) throw new InputError("request too large");
  return text;
}

async function handlePage(request, store, base, api) {
  if (request.method === "POST") {
    const form = new URLSearchParams(await readBody(request));
    await store.add({ body: form.get("body") ?? "", sender: form.get("sender"), direction: "inbound", source: "page" });
    return new Response(null, { status: 303, headers: { location: base } });
  }
  if (request.method !== "GET" && request.method !== "HEAD") {
    return new Response("Method Not Allowed", { status: 405, headers: { allow: "GET, POST" } });
  }
  const { messages, stats } = await store.list({ direction: "all", order: "desc", limit: 200 });
  const html = renderPage({ messages, stats, basePath: base, apiPath: api });
  return new Response(html, {
    headers: { "content-type": "text/html; charset=utf-8", "content-security-policy": PAGE_CSP },
  });
}

async function handleApi(request, store, rest, url) {
  const [what, ...more] = rest;
  if (more.length) return notFound();
  if (what === "messages") {
    if (request.method === "GET") {
      const p = url.searchParams;
      return json(
        await store.list({
          direction: p.get("direction") ?? "all",
          since_id: p.get("since_id") ?? undefined,
          unread_only: p.get("unread_only") === "true" || p.get("unread_only") === "1",
          limit: p.get("limit") ?? undefined,
          order: p.get("order") ?? undefined,
        }),
      );
    }
    if (request.method === "POST") {
      let data;
      try {
        data = JSON.parse(await readBody(request));
      } catch (err) {
        if (err instanceof InputError) throw err;
        throw new InputError("body must be JSON");
      }
      if (!data || typeof data !== "object") throw new InputError("body must be a JSON object");
      const source = data.source === "page" ? "page" : "api";
      const message = await store.add({ body: data.body, sender: data.sender, direction: "inbound", source });
      return json({ message }, 201);
    }
    return new Response("Method Not Allowed", { status: 405, headers: { allow: "GET, POST" } });
  }
  if (what === "ack") {
    if (request.method !== "POST") return new Response("Method Not Allowed", { status: 405, headers: { allow: "POST" } });
    let data;
    try {
      data = JSON.parse(await readBody(request));
    } catch (err) {
      if (err instanceof InputError) throw err;
      throw new InputError("body must be JSON");
    }
    return json(await store.ack({ up_to_id: data?.up_to_id, ids: data?.ids }));
  }
  return notFound();
}

export async function route(request, env, getStore) {
  const url = new URL(request.url);
  if (url.pathname === "/healthz") return json({ ok: true, configured: tokenConfigured(env) });
  const m = matchTokenPath(url.pathname, env);
  if (!m) return notFound();
  const token = env.ACCESS_TOKEN;
  const store = getStore();
  try {
    if (m.area === "mcp" && m.rest.length === 0) return withHeaders(await handleMcp(request, store));
    if (m.area === "p" && m.rest.length === 0) {
      return withHeaders(await handlePage(request, store, `/p/${token}`, `/api/${token}`));
    }
    if (m.area === "api") return withHeaders(await handleApi(request, store, m.rest, url));
  } catch (err) {
    if (err instanceof InputError) return withHeaders(json({ error: err.message }, 400));
    throw err;
  }
  return notFound();
}
