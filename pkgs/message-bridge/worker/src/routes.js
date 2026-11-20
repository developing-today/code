// Request router. Pure: takes the request, env and a message store, so
// the tests can drive it with MemoryStore and index.js with the
// Durable Object stub.
//
//   POST /mcp/<token>                 remote MCP server (Streamable HTTP),
//                                     Claude's channel
//   POST /mcp/<chatgpt token>         the same server on ChatGPT's channel,
//                                     for the ChatGPT connector's Site
//   GET  /p/<token>                   history page with the send form
//   POST /p/<token>                   form post: store an inbound message
//                                     for each checked recipient (to)
//   GET  /api/<token>/messages        JSON history (direction, channel,
//                                     kind, since_id, before_id, unread_only,
//                                     limit, order)
//   POST /api/<token>/messages        JSON {body, sender, to, kind}: store
//                                     an inbound message per recipient; to
//                                     is "claude" (default), "chatgpt" or
//                                     both; kind is "message" (default) or
//                                     "instruction" (agents see no sender)
//   POST /api/<token>/ack             JSON {up_to_id | ids, channel}: mark read
//   GET  /healthz                     {ok, configured, chatgpt_configured,
//                                     version, token_fingerprint}, no data
//   POST / and POST /mcp              the MCP server for a connector added
//                                     with just the host: OAuth bearer
//                                     token, 401 without one (oauth.js)
//   /.well-known/oauth-*, /register,  OAuth discovery, client registration,
//   /authorize, /token                approval and tokens (oauth.js)
// Anything else, a wrong path token included, is a plain 404. The
// ChatGPT token opens only its /mcp.

import { chatgptConfigured, matchTokenPath, tokenConfigured, tokenFingerprint } from "./auth.js";
import { handleMcp } from "./mcp.js";
import { MCP_CORS, bearerChannel, handleOAuth, ownerCookie, unauthorized } from "./oauth.js";
import { InputError, normalizeKind, normalizeRecipients } from "./messages.js";
import { renderPage } from "./page.js";

const MAX_FORM_BYTES = 64 * 1024;
const PAGE_SIZE = 200;

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

/** Stores one inbound message per recipient; validates all before writing. */
async function sendToEach(store, { body, sender, to, source, kind }) {
  const channels = normalizeRecipients(to);
  const k = normalizeKind(kind);
  if (typeof body !== "string" || body.trim() === "") throw new InputError("body must be a non-empty string");
  const messages = [];
  for (const channel of channels) {
    messages.push(await store.add({ body, sender, direction: "inbound", source, channel, kind: k }));
  }
  return messages;
}

async function handlePage(request, store, base, api, url, chatgptReady) {
  if (request.method === "POST") {
    const form = new URLSearchParams(await readBody(request));
    // Without JavaScript the checkboxes post one "to" each; none checked
    // falls back to Claude, as before.
    const to = form.getAll("to");
    await sendToEach(store, {
      body: form.get("body") ?? "",
      sender: form.get("sender"),
      to: to.length ? to : undefined,
      kind: form.get("kind") ?? undefined,
      source: "page",
    });
    return new Response(null, { status: 303, headers: { location: base } });
  }
  if (request.method !== "GET" && request.method !== "HEAD") {
    return new Response("Method Not Allowed", { status: 405, headers: { allow: "GET, POST" } });
  }
  const beforeId = url.searchParams.get("before_id") ?? undefined;
  const { messages, stats } = await store.list({
    direction: "all",
    order: "desc",
    limit: PAGE_SIZE,
    before_id: beforeId,
  });
  const html = renderPage({
    messages,
    stats,
    basePath: base,
    apiPath: api,
    pageSize: PAGE_SIZE,
    olderOnly: !!beforeId,
    chatgptReady,
  });
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
          channel: p.get("channel") ?? undefined,
          kind: p.get("kind") ?? undefined,
          since_id: p.get("since_id") ?? undefined,
          before_id: p.get("before_id") ?? undefined,
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
      const messages = await sendToEach(store, {
        body: data.body,
        sender: data.sender,
        to: data.to,
        kind: data.kind,
        source,
      });
      // message stays for callers from before there were two channels.
      return json({ message: messages[0], messages }, 201);
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
    return json(await store.ack({ up_to_id: data?.up_to_id, ids: data?.ids, channel: data?.channel ?? "claude" }));
  }
  return notFound();
}

export async function route(request, env, getStore) {
  const url = new URL(request.url);
  if (url.pathname === "/healthz") {
    const configured = tokenConfigured(env);
    return json({
      ok: true,
      configured,
      chatgpt_configured: chatgptConfigured(env),
      version: env.CF_VERSION_METADATA?.id ?? null,
      token_fingerprint: configured ? await tokenFingerprint(env.ACCESS_TOKEN) : null,
    });
  }
  const oauth = await handleOAuth(request, env, url, getStore);
  if (oauth) return oauth;
  if (["/", "/mcp", "/mcp/"].includes(url.pathname) && tokenConfigured(env)) {
    if (request.method === "OPTIONS") return new Response(null, { status: 204, headers: MCP_CORS });
    const channel = await bearerChannel(request, env);
    if (!channel) return unauthorized(request, url);
    try {
      return withHeaders(await handleMcp(request, getStore(), channel), MCP_CORS);
    } catch (err) {
      if (err instanceof InputError) return withHeaders(json({ error: err.message }, 400), MCP_CORS);
      throw err;
    }
  }
  const m = matchTokenPath(url.pathname, env);
  if (!m) return notFound();
  const token = env.ACCESS_TOKEN;
  const store = getStore();
  try {
    if (m.area === "mcp" && m.rest.length === 0) return withHeaders(await handleMcp(request, store, m.channel));
    if (m.role !== "owner") return notFound();
    if (m.area === "p" && m.rest.length === 0) {
      const res = withHeaders(
        await handlePage(request, store, `/p/${token}`, `/api/${token}`, url, chatgptConfigured(env)),
      );
      // Opening the page marks this browser as the owner's, so approving
      // a connector here (oauth.js) is one tap.
      if (request.method === "GET" && res.status === 200) res.headers.append("set-cookie", await ownerCookie(env));
      return res;
    }
    if (m.area === "api") return withHeaders(await handleApi(request, store, m.rest, url));
  } catch (err) {
    if (err instanceof InputError) return withHeaders(json({ error: err.message }, 400));
    throw err;
  }
  return notFound();
}
