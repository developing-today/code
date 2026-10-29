#!/usr/bin/env node
// OpenCode Route Proxy: an OpenAI-compatible front for the fleet OpenCode
// server, so OpenAI-chat-completions clients (OpenClaw's models.providers
// in particular) can run turns on opencode-route/* models — above all
// opencode-route/workhorse. #2229 mechanism 3.
//
// Pattern mirrors the other bin/* provider bridges (commandcode-proxy,
// codex-proxy): loopback only, no API-key check (callers use the
// "local-proxy" placeholder), port parsed strictly via proxy-port.mjs.
//
// Semantic, stated plainly: this is an agent-behind-a-completions-facade,
// like the CLI bridges. Each chat-completions request becomes ONE fresh
// OpenCode session on the standing server (127.0.0.1:13420, v2 API):
// create (route model set at creation) -> prompt -> wait -> read ->
// delete. The conversation is flattened into a single transcript prompt;
// the OpenCode build agent then does the work with OpenCode's own tools
// and its final assistant text is the completion. OpenAI-style tool_calls
// are NOT round-tripped — callers that need their own tool loop should
// not use this front; callers that want a job done by the route should.
//
// The OpenCode server password is read at runtime from
// ~/.local/state/opencode/service-prod.json (re-read on a 401); it is
// never logged or committed.
//
// Env:
//   OPENCODE_ROUTE_PROXY_PORT        listen port (default 18750)
//   OPENCODE_ROUTE_PROXY_HOST        listen host (default 127.0.0.1)
//   OPENCODE_ROUTE_PROXY_DIR         session directory (default /home/user)
//   OPENCODE_ROUTE_PROXY_SERVER      OpenCode base URL (default http://127.0.0.1:13420)
//   OPENCODE_ROUTE_PROXY_TIMEOUT_MS  per-request cap (default 1200000)
//   OPENCODE_ROUTE_PROXY_KEEP_SESSIONS  "1" keeps sessions for inspection
import { createServer } from "node:http";
import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { parseProxyPort } from "./proxy-port.mjs";

const PORT = parseProxyPort("OPENCODE_ROUTE_PROXY_PORT", 18750);
const HOST = process.env.OPENCODE_ROUTE_PROXY_HOST || "127.0.0.1";
const DIRECTORY = process.env.OPENCODE_ROUTE_PROXY_DIR || "/home/user";
const SERVER = (
  process.env.OPENCODE_ROUTE_PROXY_SERVER || "http://127.0.0.1:13420"
).replace(/\/$/, "");
const TIMEOUT_MS = Number(process.env.OPENCODE_ROUTE_PROXY_TIMEOUT_MS || 1_200_000);
const KEEP_SESSIONS = process.env.OPENCODE_ROUTE_PROXY_KEEP_SESSIONS === "1";
const SERVICE_FILE =
  process.env.OPENCODE_SERVICE_FILE ||
  join(homedir(), ".local/state/opencode/service-prod.json");

// The route models this front serves. Any other opencode-route/* id is
// passed through as-is (the server rejects unknown routes); these are
// the ones advertised on /v1/models.
const ADVERTISED_MODELS = [
  "workhorse",
  "speed",
  "specialist",
  "summit",
  "scout",
  "workhorse-weighted",
];

let cachedPassword = null;
const readPassword = () => {
  const data = JSON.parse(readFileSync(SERVICE_FILE, "utf8"));
  if (!data.password) throw new Error(`no password in ${SERVICE_FILE}`);
  cachedPassword = data.password;
  return cachedPassword;
};
const authHeader = () =>
  "Basic " +
  Buffer.from(`opencode:${cachedPassword ?? readPassword()}`).toString("base64");

const ocFetch = async (path, init = {}, retry = true) => {
  const res = await fetch(`${SERVER}${path}`, {
    ...init,
    headers: {
      authorization: authHeader(),
      "x-opencode-directory": DIRECTORY,
      "content-type": "application/json",
      ...(init.headers || {}),
    },
  });
  if (res.status === 401 && retry) {
    readPassword();
    return ocFetch(path, init, false);
  }
  return res;
};

const ocJson = async (path, init) => {
  const res = await ocFetch(path, init);
  const text = await res.text();
  let body = null;
  try {
    body = text ? JSON.parse(text) : null;
  } catch {
    /* non-JSON body: surfaced via status below */
  }
  if (!res.ok) {
    throw new Error(
      `OpenCode ${init?.method || "GET"} ${path} -> ${res.status}: ${text.slice(0, 300)}`,
    );
  }
  return body?.data ?? body;
};

// Flatten an OpenAI messages array into one transcript prompt. Tool
// results and prior assistant turns are preserved as labelled text so a
// stateless caller still gets its context into the session.
const flattenMessages = (messages) => {
  const parts = [];
  for (const m of messages || []) {
    const role = m.role || "user";
    let content = "";
    if (typeof m.content === "string") content = m.content;
    else if (Array.isArray(m.content))
      content = m.content
        .map((p) => (typeof p?.text === "string" ? p.text : ""))
        .filter(Boolean)
        .join("\n");
    if (m.role === "tool" || m.role === "function") {
      parts.push(`[tool result${m.name ? ` from ${m.name}` : ""}]\n${content}`);
    } else if (role === "assistant") {
      if (content) parts.push(`[assistant previously replied]\n${content}`);
      for (const tc of m.tool_calls || [])
        parts.push(
          `[assistant previously called tool ${tc?.function?.name || "unknown"} with arguments ${tc?.function?.arguments || ""}]`,
        );
    } else if (role === "system") {
      parts.push(`[system instructions]\n${content}`);
    } else {
      parts.push(content);
    }
  }
  return parts.join("\n\n").trim();
};

const parseRouteModel = (raw) => {
  const model = typeof raw === "string" && raw ? raw : "workhorse";
  return model.startsWith("opencode-route/")
    ? model.slice("opencode-route/".length)
    : model;
};

// Run one completion: a fresh OpenCode session on the requested route.
const runCompletion = async (routeId, promptText, onText) => {
  const session = await ocJson("/api/session", {
    method: "POST",
    body: JSON.stringify({
      title: `opencode-route-proxy: ${routeId}`,
      model: { providerID: "opencode-route", id: routeId },
    }),
  });
  const sessionId = session?.id;
  if (!sessionId) throw new Error("OpenCode session create returned no id");
  try {
    const prompted = await ocFetch(`/api/session/${sessionId}/prompt`, {
      method: "POST",
      body: JSON.stringify({ text: promptText }),
    });
    if (!prompted.ok)
      throw new Error(
        `OpenCode prompt -> ${prompted.status}: ${(await prompted.text()).slice(0, 300)}`,
      );

    // Wait for the turn to settle, streaming text growth to the caller
    // when a sink is provided.
    const deadline = Date.now() + TIMEOUT_MS;
    let lastText = "";
    let finalMessage = null;
    while (Date.now() < deadline) {
      const waitRes = await ocFetch(
        `/api/experimental/session/${sessionId}/wait`,
        { method: "POST" },
      ).catch(() => null);
      const messages = await ocJson(`/api/session/${sessionId}/message`).catch(
        () => null,
      );
      const list = Array.isArray(messages) ? messages : [];
      const assistant = list.find((m) => m.type === "assistant");
      const text = (assistant?.content || [])
        .filter((p) => p.type === "text")
        .map((p) => p.text)
        .join("");
      if (text && text !== lastText) {
        lastText = text;
        if (onText) onText(text);
      }
      if (assistant && assistant.finish) {
        finalMessage = assistant;
        break;
      }
      if (waitRes && waitRes.status === 204) {
        // Settled; one last read decides whether we truly have a finish.
        const again = await ocJson(`/api/session/${sessionId}/message`).catch(
          () => null,
        );
        const a2 = (Array.isArray(again) ? again : []).find(
          (m) => m.type === "assistant",
        );
        if (a2 && a2.finish) {
          finalMessage = a2;
          break;
        }
        if (a2 && !assistant) {
          finalMessage = a2;
          break;
        }
      }
      await new Promise((r) => setTimeout(r, 1000));
    }
    if (!finalMessage) {
      await ocFetch(`/api/session/${sessionId}/interrupt`, {
        method: "POST",
      }).catch(() => {});
      throw new Error(`OpenCode session ${sessionId} did not finish within ${TIMEOUT_MS}ms`);
    }
    const text = (finalMessage.content || [])
      .filter((p) => p.type === "text")
      .map((p) => p.text)
      .join("");
    return { text, sessionId, tokens: finalMessage.tokens || {} };
  } finally {
    if (!KEEP_SESSIONS)
      await ocFetch(`/api/session/${sessionId}`, { method: "DELETE" }).catch(
        () => {},
      );
  }
};

const sendJson = (res, status, obj) => {
  const body = JSON.stringify(obj);
  res.writeHead(status, { "content-type": "application/json" });
  res.end(body);
};
const sendError = (res, status, message) =>
  sendJson(res, status, { error: { message, type: "proxy_error" } });

const handleChatCompletions = async (req, res, body) => {
  const routeId = parseRouteModel(body.model);
  const promptText = flattenMessages(body.messages);
  if (!promptText) return sendError(res, 400, "no message content");
  const completionId = `chatcmpl-${Date.now().toString(36)}`;
  const modelName = `opencode-route/${routeId}`;

  if (body.stream) {
    res.writeHead(200, {
      "content-type": "text/event-stream",
      "cache-control": "no-cache",
      connection: "keep-alive",
    });
    const chunk = (delta, finishReason = null) =>
      res.write(
        `data: ${JSON.stringify({
          id: completionId,
          object: "chat.completion.chunk",
          created: Math.floor(Date.now() / 1000),
          model: modelName,
          choices: [
            { index: 0, delta, finish_reason: finishReason },
          ],
        })}\n\n`,
      );
    chunk({ role: "assistant", content: "" });
    let sent = "";
    try {
      const { text, tokens } = await runCompletion(routeId, promptText, (full) => {
        if (full.startsWith(sent)) {
          const delta = full.slice(sent.length);
          if (delta) chunk({ content: delta });
          sent = full;
        }
      });
      if (text && !text.startsWith(sent)) chunk({ content: text });
      else if (text.length > sent.length)
        chunk({ content: text.slice(sent.length) });
      chunk({}, "stop");
      res.write(
        `data: ${JSON.stringify({
          id: completionId,
          object: "chat.completion.chunk",
          created: Math.floor(Date.now() / 1000),
          model: modelName,
          choices: [],
          usage: {
            prompt_tokens: tokens.input || 0,
            completion_tokens: tokens.output || 0,
            total_tokens: (tokens.input || 0) + (tokens.output || 0),
          },
        })}\n\n`,
      );
      res.write("data: [DONE]\n\n");
      res.end();
    } catch (err) {
      res.write(
        `data: ${JSON.stringify({ error: { message: String(err?.message || err), type: "proxy_error" } })}\n\n`,
      );
      res.end();
    }
    return;
  }

  try {
    const { text, tokens } = await runCompletion(routeId, promptText, null);
    sendJson(res, 200, {
      id: completionId,
      object: "chat.completion",
      created: Math.floor(Date.now() / 1000),
      model: modelName,
      choices: [
        {
          index: 0,
          message: { role: "assistant", content: text },
          finish_reason: "stop",
        },
      ],
      usage: {
        prompt_tokens: tokens.input || 0,
        completion_tokens: tokens.output || 0,
        total_tokens: (tokens.input || 0) + (tokens.output || 0),
      },
    });
  } catch (err) {
    sendError(res, 502, String(err?.message || err));
  }
};

const server = createServer(async (req, res) => {
  try {
    const url = new URL(req.url, `http://${HOST}:${PORT}`);
    if (req.method === "GET" && url.pathname === "/health") {
      return sendJson(res, 200, { ok: true, service: "opencode-route-proxy" });
    }
    if (req.method === "GET" && url.pathname === "/v1/models") {
      return sendJson(res, 200, {
        object: "list",
        data: ADVERTISED_MODELS.map((id) => ({
          id,
          object: "model",
          created: 0,
          owned_by: "opencode-route",
        })),
      });
    }
    if (req.method === "POST" && url.pathname === "/v1/chat/completions") {
      let raw = "";
      for await (const part of req) raw += part;
      let body = {};
      try {
        body = JSON.parse(raw || "{}");
      } catch {
        return sendError(res, 400, "invalid JSON body");
      }
      return handleChatCompletions(req, res, body);
    }
    return sendError(res, 404, `no route for ${req.method} ${url.pathname}`);
  } catch (err) {
    return sendError(res, 500, String(err?.message || err));
  }
});

server.listen(PORT, HOST, () => {
  console.log(`opencode-route-proxy listening on http://${HOST}:${PORT}`);
});
