// Request router. Pure: takes the request, env and a message store, so
// the tests can drive it with MemoryStore and index.js with the
// Durable Object stub.
//
//   POST /mcp/<token>                 remote MCP server (Streamable HTTP),
//                                     Claude's channel
//   POST /mcp/<chatgpt token>         the same server on ChatGPT's channel,
//                                     with MCP Events (events.js) so a
//                                     message for ChatGPT wakes its chat
//   GET  /p/<token>                   history page with the send form
//   POST /p/<token>                   form post: store an inbound message
//                                     for each checked recipient (to)
//   GET  /api/<token>/messages        JSON history (direction, channel,
//                                     kind, since_id, before_id, unread_only,
//                                     limit, order)
//   GET  /api/<token>/pipeline        one normalized feed over connector
//                                     messages, fleet messages and notes
//                                     (pipeline.js; kind, sender, receiver,
//                                     surface, batch, unread_only,
//                                     since_id, before_id, limit, order)
//   GET  /api/<token>/notes           owner read over the notes table
//                                     (pipeline.js; the same filters)
//   GET  /api/<token>/agents          unified connected-agents list:
//                                     fleet registry + note identities,
//                                     each row marked by surface
//   POST /api/<token>/messages        JSON {body, sender, to, kind}: store
//                                     an inbound message per recipient; to
//                                     is "claude" (default), "chatgpt" or
//                                     both; kind is "message" (default) or
//                                     "instruction" (agents see no sender)
//   POST /api/<token>/ack             JSON {up_to_id | ids, channel}: mark read
//   GET  /api/<token>/oauth-events    the last connector sign-in events
//                                     (registrations, approvals, refusals;
//                                     no secrets) and ChatGPT event
//                                     deliveries, newest first
//   GET  /api/<token>/event-subscriptions  live ChatGPT event subscriptions
//                                     (host and expiry only, events.js)
//   GET  /api/<token>/wake-hooks      wake hooks (address, kind, host;
//                                     never the token or path, wake.js)
//   POST /api/<token>/wake-hooks      JSON {address, url, token?} sets one;
//                                     {address, url: null} removes it
//   POST /fleet/<token>/hello|inbox|ack|send  fleet mailboxes (fleet.js);
//                                     the token is the shared fleet token
//                                     or one agent's own credential
//                                     (#14368), which acts only as itself
//   POST /api/<token>/enroll-tokens   owner: mint a bootstrap enroll
//                                     token bound to a name/kind/groups;
//                                     the value is returned once
//   POST /api/<token>/enroll-tokens/revoke  owner: revoke an unused
//                                     bootstrap token
//   POST /enroll/exchange             {bootstrap_token, agent}: burn the
//                                     token, register the mailbox (and
//                                     notes identity), return the agent's
//                                     fleet credential + notes token once
//   POST /api/<token>/fleet/credentials/revoke  owner: revoke an agent's
//                                     fleet credential(s)
//   POST /api/<token>/notes-tokens/revoke  owner: revoke an identity's
//                                     notes token(s)
//   POST /notes/register              owner Bearer token; registers a
//                                     class/name identity + token hash
//   POST /notes/send                  identity Bearer token; {to, note},
//                                     to one identity or a */segment match
//   GET  /notes/inbox                 identity Bearer token; its pending
//                                     notes
//   POST /notes/ack                   identity Bearer token; {id|ids|
//                                     up_to_id} acknowledges its notes
//   GET  /<class>/<name>              public wake signal (notes.js): one
//                                     bare UUID, same shape registered
//                                     or not, decoy-rotated per path
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
import { handleFleet } from "./fleet.js";
import { handleMcp } from "./mcp.js";
import { EVENT_CHANNELS, deliverMessages, eventsFor } from "./events.js";
import { MCP_CORS, bearerChannel, handleOAuth, ownerCookie, unauthorized } from "./oauth.js";
import { InputError, normalizeKind, normalizeRecipients } from "./messages.js";
import { renderPage } from "./page.js";
import { normalizeHook, showHook, wakeAll, WAKE_ADDRESS } from "./wake.js";
import { handleNotes, handleSignal } from "./notes.js";
import { notesOwnerFeed, pipelineFeed, unifiedAgents } from "./pipeline.js";

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

/** Logs to the owner's sign-in/event log; logging never breaks a request. */
const logTo = (store) => async (kind, detail) => {
  try {
    await store.oauthLog(kind, detail);
  } catch {
    // diagnostics only
  }
};

/** Sets ({address, url, token}) or, with no url, removes a wake hook. */
async function setWakeHook(store, { address, url, token }) {
  if (url === undefined || url === null || url === "") {
    const a = typeof address === "string" ? address.trim().toLowerCase() : "";
    if (!WAKE_ADDRESS.test(a)) throw new InputError("address must be channel:claude, channel:chatgpt or agent:<name>");
    return { removed: await store.wakeHookDelete(a) };
  }
  const hook = { ...normalizeHook({ address, url, token }), added: Date.now() };
  await store.wakeHookPut(hook);
  return { hook: showHook(hook) };
}

async function handlePage(request, store, base, api, url, chatgptReady, delivered) {
  if (request.method === "POST") {
    const form = new URLSearchParams(await readBody(request));
    if (form.get("action") === "wake_hook") {
      await setWakeHook(store, {
        address: form.get("address") ?? "",
        url: form.get("remove") ? null : form.get("url"),
        token: form.get("token"),
      });
      return new Response(null, { status: 303, headers: { location: base } });
    }
    // Without JavaScript the checkboxes post one "to" each; none checked
    // falls back to Claude, as before.
    const to = form.getAll("to");
    const sent = await sendToEach(store, {
      body: form.get("body") ?? "",
      sender: form.get("sender"),
      to: to.length ? to : undefined,
      kind: form.get("kind") ?? undefined,
      source: "page",
    });
    delivered(sent);
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
    wakeHooks: (await store.wakeHooks()).map(showHook),
    // The page's own token, so the address is only ever shown to its owner.
    connectorUrl: `${url.origin}/mcp/${base.slice("/p/".length)}`,
  });
  return new Response(html, {
    headers: { "content-type": "text/html; charset=utf-8", "content-security-policy": PAGE_CSP },
  });
}

async function handleApi(request, store, rest, url, delivered) {
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
      delivered(messages);
      // message stays for callers from before there were two channels.
      return json({ message: messages[0], messages }, 201);
    }
    return new Response("Method Not Allowed", { status: 405, headers: { allow: "GET, POST" } });
  }
  if (what === "pipeline") {
    if (request.method !== "GET") return new Response("Method Not Allowed", { status: 405, headers: { allow: "GET" } });
    return json(await pipelineFeed(store, Object.fromEntries(url.searchParams.entries())));
  }
  if (what === "notes") {
    if (request.method !== "GET") return new Response("Method Not Allowed", { status: 405, headers: { allow: "GET" } });
    return json(await notesOwnerFeed(store, Object.fromEntries(url.searchParams.entries())));
  }
  if (what === "agents") {
    if (request.method !== "GET") return new Response("Method Not Allowed", { status: 405, headers: { allow: "GET" } });
    return json(await unifiedAgents(store));
  }
  if (what === "oauth-events") {
    if (request.method !== "GET") return new Response("Method Not Allowed", { status: 405, headers: { allow: "GET" } });
    return json({ events: await store.oauthEvents() });
  }
  if (what === "event-subscriptions") {
    if (request.method !== "GET") return new Response("Method Not Allowed", { status: 405, headers: { allow: "GET" } });
    const subs = [];
    for (const channel of EVENT_CHANNELS) {
      for (const sub of await store.eventSubsLive(channel, Date.now())) {
        subs.push({
          id: sub.id,
          channel,
          host: new URL(sub.url).hostname,
          created: new Date(sub.created).toISOString(),
          expires: new Date(sub.expires).toISOString(),
        });
      }
    }
    return json({ subscriptions: subs });
  }
  if (what === "wake-hooks") {
    if (request.method === "GET") return json({ hooks: (await store.wakeHooks()).map(showHook) });
    if (request.method !== "POST") return new Response("Method Not Allowed", { status: 405, headers: { allow: "GET, POST" } });
    let data;
    try {
      data = JSON.parse(await readBody(request));
    } catch (err) {
      if (err instanceof InputError) throw err;
      throw new InputError("body must be JSON");
    }
    return json(await setWakeHook(store, data ?? {}));
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

/**
 * What happens after rows are stored: ChatGPT's MCP Events and every
 * wake hook (wake.js) for the addresses that got one. Takes the rows as
 * an array of connector rows or as {messages, fleet}. Never throws.
 */
export function deliveredFor(env, store, waitUntil) {
  return (out) => {
    const o = Array.isArray(out) ? { messages: out, fleet: [] } : (out ?? {});
    const log = logTo(store);
    waitUntil(
      Promise.all([
        deliverMessages(env, store, o.messages ?? [], log).catch(() => {}),
        wakeAll(env, store, { messages: o.messages ?? [], fleet: o.fleet ?? [] }, log).catch(() => {}),
      ]),
    );
  };
}

/**
 * waitUntil keeps work going after the response (event deliveries); the
 * Worker passes ctx.waitUntil, and tests can collect the promises.
 */
export async function route(request, env, getStore, waitUntil = (p) => p) {
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
      const store = getStore();
      return withHeaders(
        await handleMcp(request, store, channel, eventsFor(env, store, channel, logTo(store)), deliveredFor(env, store, waitUntil)),
        MCP_CORS,
      );
    } catch (err) {
      if (err instanceof InputError) return withHeaders(json({ error: err.message }, 400), MCP_CORS);
      throw err;
    }
  }
  // Fleet mailboxes: /fleet/<fleet token>/... and /api/<owner>/fleet/...
  const fleet = await handleFleet(request, env, url, getStore, (store) => deliveredFor(env, store, waitUntil));
  if (fleet) return fleet;
  // Class/name notes (notes.js): the authenticated /notes API, then the
  // public content-free signal on any other two-segment path.
  const notes = await handleNotes(request, env, url, getStore);
  if (notes) return notes;
  const signal = await handleSignal(request, url, getStore);
  if (signal) return signal;
  const m = matchTokenPath(url.pathname, env);
  if (!m) return notFound();
  const token = env.ACCESS_TOKEN;
  const store = getStore();
  const delivered = deliveredFor(env, store, waitUntil);
  try {
    if (m.area === "mcp" && m.rest.length === 0) {
      return withHeaders(
        await handleMcp(request, store, m.channel, eventsFor(env, store, m.channel, logTo(store)), delivered),
      );
    }
    if (m.role !== "owner") return notFound();
    if (m.area === "p" && m.rest.length === 0) {
      const res = withHeaders(
        await handlePage(request, store, `/p/${token}`, `/api/${token}`, url, chatgptConfigured(env), delivered),
      );
      // Opening the page marks this browser as the owner's, so approving
      // a connector here (oauth.js) is one tap.
      if (request.method === "GET" && res.status === 200) res.headers.append("set-cookie", await ownerCookie(env));
      return res;
    }
    if (m.area === "api") return withHeaders(await handleApi(request, store, m.rest, url, delivered));
  } catch (err) {
    if (err instanceof InputError) return withHeaders(json({ error: err.message }, 400));
    throw err;
  }
  return notFound();
}
