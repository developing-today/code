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
// The whole request is bounded by OPENCODE_ROUTE_PROXY_TIMEOUT_MS: one
// deadline aborts every upstream call (create, prompt, wait, reads), so
// a stalled server cannot hold a request past the cap. A client that
// disconnects mid-turn aborts the same way, and the upstream session is
// interrupted and deleted rather than left running its tools. A turn
// the server reports as finish:"error" is surfaced as a proxy error,
// never as a successful empty completion, so callers can fall back.
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

// Error text from an upstream error value, mirroring the aggregator's
// v2 mapper (pm-stack/aggregator/src/sources/opencode.ts): the value is
// a string, or an object carrying message/type.
const errText = (e) => {
  if (!e) return null;
  if (typeof e === "string") return e;
  if (typeof e === "object") {
    if (typeof e.message === "string") return e.message;
    if (typeof e.type === "string") return e.type;
  }
  return String(e);
};

// The failure text for a finish:"error" turn: the error rides on the
// assistant message itself or on an accompanying message (the v2 shape
// carries quota-exhaustion on an idle message next to the failed one).
const turnErrorText = (messages, assistant) => {
  const own = errText(assistant?.error);
  if (own) return own;
  for (const m of messages || []) {
    const t = errText(m?.error);
    if (t) return t;
  }
  return "turn failed (finish=error)";
};

// OpenAI finish_reason for an upstream finish value: only "length" is
// meaningful to pass through; terminal values otherwise read as "stop".
const mapFinishReason = (finish) => (finish === "length" ? "length" : "stop");

// A message with finish "tool-calls" is mid-turn, not a final answer.
const isFinalFinish = (m) =>
  Boolean(m && m.finish && m.finish !== "tool-calls");

const abortableSleep = (ms, signal) =>
  new Promise((resolve) => {
    if (signal.aborted) return resolve();
    const t = setTimeout(finish, ms);
    function finish() {
      clearTimeout(t);
      resolve();
    }
    signal.addEventListener("abort", finish, { once: true });
  });

// Run one completion: a fresh OpenCode session on the requested route.
// One deadline (TIMEOUT_MS) aborts every upstream call, and the
// caller's signal (client disconnect) aborts the same way; either abort
// interrupts and deletes the upstream session instead of leaving its
// tool execution running.
const runCompletion = async (routeId, promptText, onText, clientSignal) => {
  const deadlineCtl = new AbortController();
  const timer = setTimeout(
    () => deadlineCtl.abort(new Error("opencode-route-proxy deadline")),
    TIMEOUT_MS,
  );
  const signal = clientSignal
    ? AbortSignal.any([deadlineCtl.signal, clientSignal])
    : deadlineCtl.signal;
  try {
    const session = await ocJson("/api/session", {
      method: "POST",
      body: JSON.stringify({
        title: `opencode-route-proxy: ${routeId}`,
        model: { providerID: "opencode-route", id: routeId },
      }),
      signal,
    });
    const sessionId = session?.id;
    if (!sessionId) throw new Error("OpenCode session create returned no id");
    try {
      const prompted = await ocFetch(`/api/session/${sessionId}/prompt`, {
        method: "POST",
        body: JSON.stringify({ text: promptText }),
        signal,
      });
      if (!prompted.ok)
        throw new Error(
          `OpenCode prompt -> ${prompted.status}: ${(await prompted.text()).slice(0, 300)}`,
        );

      // Wait for the turn to settle, streaming text growth to the caller
      // when a sink is provided.
      let lastText = "";
      let finalMessage = null;
      let lastList = [];
      while (!signal.aborted) {
        const waitRes = await ocFetch(
          `/api/experimental/session/${sessionId}/wait`,
          { method: "POST", signal },
        ).catch(() => null);
        const messages = await ocJson(`/api/session/${sessionId}/message`, {
          signal,
        }).catch(() => null);
        const list = Array.isArray(messages) ? messages : [];
        if (messages) lastList = list;
        const assistant = list.find((m) => m.type === "assistant");
        const text = (assistant?.content || [])
          .filter((p) => p.type === "text")
          .map((p) => p.text)
          .join("");
        if (text && text !== lastText) {
          lastText = text;
          if (onText) onText(text);
        }
        if (isFinalFinish(assistant)) {
          finalMessage = assistant;
          break;
        }
        if (waitRes && waitRes.status === 204) {
          // Settled; one last read decides whether we truly have a finish.
          const again = await ocJson(`/api/session/${sessionId}/message`, {
            signal,
          }).catch(() => null);
          const againList = Array.isArray(again) ? again : [];
          if (again) lastList = againList;
          const a2 = againList.find((m) => m.type === "assistant");
          if (isFinalFinish(a2)) {
            finalMessage = a2;
            break;
          }
          if (a2 && !assistant) {
            finalMessage = a2;
            break;
          }
        }
        await abortableSleep(1000, signal);
      }
      if (!finalMessage) {
        throw new Error(
          clientSignal?.aborted
            ? "client disconnected before the OpenCode turn finished"
            : `OpenCode session ${sessionId} did not finish within ${TIMEOUT_MS}ms`,
        );
      }
      if (finalMessage.finish === "error") {
        throw new Error(
          `OpenCode turn failed: ${turnErrorText(lastList, finalMessage)}`,
        );
      }
      const text = (finalMessage.content || [])
        .filter((p) => p.type === "text")
        .map((p) => p.text)
        .join("");
      return {
        text,
        sessionId,
        tokens: finalMessage.tokens || {},
        finish: finalMessage.finish,
      };
    } catch (err) {
      // Whatever went wrong (deadline, disconnect, upstream failure),
      // stop any work still running upstream before the session goes.
      await ocFetch(`/api/session/${sessionId}/interrupt`, {
        method: "POST",
        signal: AbortSignal.timeout(5000),
      }).catch(() => {});
      throw err;
    } finally {
      if (!KEEP_SESSIONS)
        await ocFetch(`/api/session/${sessionId}`, {
          method: "DELETE",
          signal: AbortSignal.timeout(5000),
        }).catch(() => {});
    }
  } finally {
    clearTimeout(timer);
  }
};

const sendJson = (res, status, obj) => {
  if (res.destroyed || res.writableEnded) return;
  const body = JSON.stringify(obj);
  res.writeHead(status, { "content-type": "application/json" });
  res.end(body);
};
const sendError = (res, status, message) =>
  sendJson(res, status, { error: { message, type: "proxy_error" } });

const isPlainObject = (v) =>
  typeof v === "object" && v !== null && !Array.isArray(v);

const handleChatCompletions = async (req, res, body) => {
  // Shape validation: malformed (but syntactically valid) bodies get a
  // client error, never an exception out of this handler.
  if (!isPlainObject(body))
    return sendError(res, 400, "request body must be a JSON object");
  if (!Array.isArray(body.messages))
    return sendError(res, 400, "messages must be an array");
  if (body.messages.some((m) => !isPlainObject(m)))
    return sendError(res, 400, "each message must be an object");
  const routeId = parseRouteModel(body.model);
  const promptText = flattenMessages(body.messages);
  if (!promptText) return sendError(res, 400, "no message content");
  const completionId = `chatcmpl-${Date.now().toString(36)}`;
  const modelName = `opencode-route/${routeId}`;
  // A client that goes away mid-turn aborts the upstream session.
  const clientCtl = new AbortController();
  res.on("close", () => {
    if (!res.writableEnded)
      clientCtl.abort(new Error("client disconnected"));
  });

  if (body.stream) {
    res.writeHead(200, {
      "content-type": "text/event-stream",
      "cache-control": "no-cache",
      connection: "keep-alive",
    });
    const chunk = (delta, finishReason = null) => {
      if (res.destroyed || res.writableEnded) return;
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
    };
    chunk({ role: "assistant", content: "" });
    let sent = "";
    try {
      const { text, tokens, finish } = await runCompletion(
        routeId,
        promptText,
        (full) => {
          if (full.startsWith(sent)) {
            const delta = full.slice(sent.length);
            if (delta) chunk({ content: delta });
            sent = full;
          }
        },
        clientCtl.signal,
      );
      if (text && !text.startsWith(sent)) chunk({ content: text });
      else if (text.length > sent.length)
        chunk({ content: text.slice(sent.length) });
      chunk({}, mapFinishReason(finish));
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
      if (!res.destroyed && !res.writableEnded) {
        res.write(
          `data: ${JSON.stringify({ error: { message: String(err?.message || err), type: "proxy_error" } })}\n\n`,
        );
        res.end();
      }
    }
    return;
  }

  try {
    const { text, tokens, finish } = await runCompletion(
      routeId,
      promptText,
      null,
      clientCtl.signal,
    );
    sendJson(res, 200, {
      id: completionId,
      object: "chat.completion",
      created: Math.floor(Date.now() / 1000),
      model: modelName,
      choices: [
        {
          index: 0,
          message: { role: "assistant", content: text },
          finish_reason: mapFinishReason(finish),
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
  // A dead client socket must never take the proxy down with it.
  res.on("error", () => {});
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
      // Awaited inside the try: a rejected handler lands in the catch
      // below (client error / 500), never as an unhandled rejection
      // that terminates the process.
      return await handleChatCompletions(req, res, body);
    }
    return sendError(res, 404, `no route for ${req.method} ${url.pathname}`);
  } catch (err) {
    return sendError(res, 500, String(err?.message || err));
  }
});

server.listen(PORT, HOST, () => {
  console.log(`opencode-route-proxy listening on http://${HOST}:${PORT}`);
});
