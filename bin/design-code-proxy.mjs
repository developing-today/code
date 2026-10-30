#!/usr/bin/env node
import http from 'node:http';
import https from 'node:https';
import { parseProxyPort } from './proxy-port.mjs';

const PORT = parseProxyPort('DESIGN_CODE_PROXY_PORT', 18746);
const HOST = process.env.DESIGN_CODE_PROXY_HOST || '127.0.0.1';

const ANTIGRAVITY_LS_PORT = parseProxyPort('ANTIGRAVITY_LS_PROXY_PORT', 18745);
const CODEX_PROXY_PORT = parseProxyPort('CODEX_PROXY_PORT', 18744);
const AMR_UPSTREAM_URL = process.env.AMR_UPSTREAM_URL || 'https://amr-link.open-design.ai/v1';

// Warm connections (#480): one keep-alive pool per scheme, shared by every
// backend/upstream call, so repeat requests reuse an open socket instead of
// paying a fresh TCP (and, for the AMR upstream, TLS) handshake per request.
const httpAgent = new http.Agent({ keepAlive: true, keepAliveMsecs: 1000, maxSockets: 32 });
const httpsAgent = new https.Agent({ keepAlive: true, keepAliveMsecs: 1000, maxSockets: 32 });

const DESIGN_SYSTEM_PROMPT = `You are an elite Code & Systems Design Architect.
Your core expertise is in software architecture, clean code design, UI/UX design engineering, design tokens, component abstractions, domain-driven design, and maintainable systems patterns. Provide elegant, production-grade architectures and idiomatic design implementations.`;

// Model list sourcing (live-model rule, 2026-10-28 — DOCUMENTED
// EXCEPTION): this proxy's ids are VIRTUAL design personas, not upstream
// model ids — each entry is a curated routing recipe (primary/fallback
// backend + backend model) invented here, so no upstream catalog can be
// pulled for the list itself. The AMR upstream's /v1/models is API-key
// gated and no key is configured for the local-dispatch path. What CAN
// stay live does: the backend model each recipe points at is validated
// against that backend proxy's own live /v1/models (see
// refreshBackendCatalogs below), and an unknown requested id now gets a
// clean 400 instead of the old silent remap onto gemini-3.8-flash.
const MODELS = [
  { id: 'claude-haiku-5.5', name: 'Claude Haiku 5.5 (Design Code)', primary: { port: ANTIGRAVITY_LS_PORT, model: 'claude-sonnet-5-5' }, fallback: { port: ANTIGRAVITY_LS_PORT, model: 'gemini-3.8-flash' } },
  { id: 'gpt-6-luna', name: 'GPT-6 Luna (Design Code)', primary: { port: CODEX_PROXY_PORT, model: 'gpt-6-luna' }, fallback: { port: ANTIGRAVITY_LS_PORT, model: 'gemini-3.8-flash' } },
  { id: 'gpt-6.1-sol', name: 'GPT-6.1 Sol (Design Code)', primary: { port: CODEX_PROXY_PORT, model: 'gpt-6.1-sol' }, fallback: { port: ANTIGRAVITY_LS_PORT, model: 'gemini-3.8-flash' } },
  { id: 'deepseek-v4.1-flash', name: 'DeepSeek V4.1 Flash (Design Code)', primary: { port: ANTIGRAVITY_LS_PORT, model: 'gemini-3.8-flash' }, fallback: { port: CODEX_PROXY_PORT, model: 'gpt-5.6-luna' } },
  { id: 'glm-5.3-flash', name: 'GLM 5.3 Flash (Design Code)', primary: { port: ANTIGRAVITY_LS_PORT, model: 'gemini-3.8-flash' }, fallback: { port: CODEX_PROXY_PORT, model: 'gpt-5.6-luna' } },
  { id: 'glm-5.3-flashx', name: 'GLM 5.3 FlashX (Design Code)', primary: { port: ANTIGRAVITY_LS_PORT, model: 'gemini-3.8-flash' }, fallback: { port: CODEX_PROXY_PORT, model: 'gpt-5.6-luna' } },
  { id: 'mimo-v2.6-flash', name: 'MIMO v2.6 Flash (Design Code)', primary: { port: ANTIGRAVITY_LS_PORT, model: 'gemini-3.8-flash' }, fallback: { port: CODEX_PROXY_PORT, model: 'gpt-5.6-luna' } },
  { id: 'mimo-v2.6-pro', name: 'MIMO v2.6 Pro (Design Code)', primary: { port: ANTIGRAVITY_LS_PORT, model: 'gemini-3.8-flash' }, fallback: { port: CODEX_PROXY_PORT, model: 'gpt-6.1-sol' } },
  { id: 'design-code-pro', name: 'Design Code Pro', primary: { port: ANTIGRAVITY_LS_PORT, model: 'gemini-3.8-flash' }, fallback: { port: CODEX_PROXY_PORT, model: 'gpt-6.1-sol' } },
  { id: 'design-code-flash', name: 'Design Code Flash', primary: { port: ANTIGRAVITY_LS_PORT, model: 'gemini-3.8-flash' }, fallback: { port: CODEX_PROXY_PORT, model: 'gpt-5.6-luna' } },
  { id: 'open-design-pro', name: 'Open Design Pro', primary: { port: CODEX_PROXY_PORT, model: 'gpt-6.1-sol' }, fallback: { port: ANTIGRAVITY_LS_PORT, model: 'gemini-3.8-flash' } }
];

const resolveModelConfig = (requestedModel) => {
  const clean = (requestedModel || 'design-code-pro').replace(/^.*\//, '').toLowerCase().trim();
  const found = MODELS.find(m => m.id === clean || m.id.replace(/[-_.]/g, '') === clean.replace(/[-_.]/g, ''));
  if (found) {
    return { modelId: found.id, config: found };
  }

  // Fallbacks by keyword: a gpt/codex-shaped id is passed to the Codex
  // backend under its own name — and the Codex proxy now validates it
  // against its live catalog and answers a clean 400 for unknown ids.
  if (clean.includes('gpt') || clean.includes('codex') || clean.includes('luna') || clean.includes('sol')) {
    return {
      modelId: clean,
      config: {
        primary: { port: CODEX_PROXY_PORT, model: clean },
        fallback: { port: ANTIGRAVITY_LS_PORT, model: 'gemini-3.8-flash' }
      }
    };
  }

  // Unknown id: refuse (the caller answers 400) rather than silently
  // serving a different model than the one requested.
  return null;
};

// --- Live backend-catalog validation ---------------------------------------
// Background-only (never on the request path): pull each fleet backend's
// live /v1/models on a 60s TTL and record which recipe targets still
// exist upstream. Restricted to the canonical/alt fleet backend ports so
// non-fleet backend overrides are never probed. Dispatch uses the
// last-known-good result: a target proven absent upstream is skipped, and
// if every target is proven absent the request gets a clean 400.
const FLEET_BACKEND_PORTS = new Set([18744, 19744, 18745, 19745]);
const BACKEND_CATALOG_TTL_MS = 60 * 1000;
let backendCatalogCache = { at: 0, byPort: new Map() };
let backendCatalogRefresh = null;

const refreshBackendCatalogs = () => {
  if (backendCatalogRefresh) return backendCatalogRefresh;
  if (Date.now() - backendCatalogCache.at < BACKEND_CATALOG_TTL_MS && backendCatalogCache.at > 0) {
    return Promise.resolve(backendCatalogCache);
  }
  backendCatalogRefresh = (async () => {
    const ports = new Set();
    for (const m of MODELS) { ports.add(m.primary.port); if (m.fallback) ports.add(m.fallback.port); }
    const byPort = new Map(backendCatalogCache.byPort);
    for (const port of ports) {
      if (!FLEET_BACKEND_PORTS.has(port)) continue;
      try {
        const res = await fetch(`http://127.0.0.1:${port}/v1/models`, { signal: AbortSignal.timeout(2500) });
        if (!res.ok) continue;
        const data = await res.json();
        if (Array.isArray(data?.data)) byPort.set(port, new Set(data.data.map(d => d.id)));
      } catch { /* keep last-known-good for this port */ }
    }
    backendCatalogCache = { at: Date.now(), byPort };
    backendCatalogRefresh = null;
    return backendCatalogCache;
  })();
  return backendCatalogRefresh;
};

const backendTargetKnown = (target) => {
  const catalog = backendCatalogCache.byPort.get(target.port);
  if (!catalog) return null; // never pulled / not a fleet port: unvalidated
  return catalog.has(target.model);
};

const hasUpstreamKey = (authHeader) => {
  const envKey = process.env.OPEN_DESIGN_API_KEY || process.env.AMR_API_KEY;
  if (envKey && envKey.trim().length > 8) return envKey.trim();

  if (authHeader && authHeader.startsWith('Bearer ')) {
    const token = authHeader.slice(7).trim();
    if (token && !['design-local', 'local', 'none', 'dummy', 'test', 'opencode'].includes(token.toLowerCase())) {
      return token;
    }
  }
  return null;
};

const server = http.createServer((req, res) => {
  res.setHeader('Access-Control-Allow-Origin', '*');
  res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
  res.setHeader('Access-Control-Allow-Headers', '*');
  if (req.method === 'OPTIONS') {
    res.writeHead(204);
    res.end();
    return;
  }

  const url = new URL(req.url, `http://${req.headers.host || HOST}`);
  const pathname = url.pathname.replace(/\/+$/, '') || '/';

  // Health
  if (req.method === 'GET' && (pathname === '/health' || pathname === '/')) {
    refreshBackendCatalogs(); // background live validation of recipe targets
    const targetState = {};
    for (const m of MODELS) {
      targetState[m.id] = {
        primary: backendTargetKnown(m.primary),
        fallback: m.fallback ? backendTargetKnown(m.fallback) : null
      };
    }
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({
      status: 'ok',
      service: 'design-code-proxy',
      port: PORT,
      availableModels: MODELS.map(m => m.id),
      modelListSource: 'virtual routing recipes (documented exception); backend targets validated live',
      backendTargetsCheckedAt: backendCatalogCache.at ? new Date(backendCatalogCache.at).toISOString() : null,
      backendTargets: targetState
    }, null, 2));
    return;
  }

  // Models catalog
  if (req.method === 'GET' && (pathname === '/v1/models' || pathname === '/models')) {
    const now = Math.floor(Date.now() / 1000);
    const data = MODELS.map(m => ({
      id: m.id,
      object: 'model',
      created: now,
      owned_by: 'design-code'
    }));
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({ object: 'list', data }, null, 2));
    return;
  }

  // Chat completions
  if (req.method === 'POST' && (pathname === '/v1/chat/completions' || pathname === '/chat/completions')) {
    let bodyText = '';
    req.on('data', chunk => {
      bodyText += chunk;
      if (bodyText.length > 50 * 1024 * 1024) {
        res.writeHead(413, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { message: 'Payload too large' } }));
        req.destroy();
      }
    });

    req.on('end', () => {
      let body;
      try {
        body = JSON.parse(bodyText);
      } catch (err) {
        res.writeHead(400, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { message: `Invalid JSON: ${err.message}` } }));
        return;
      }

      const upstreamKey = hasUpstreamKey(req.headers.authorization);

      if (upstreamKey) {
        // Forward to AMR Link Upstream
        console.log(`[design-code-proxy] Forwarding request upstream to ${AMR_UPSTREAM_URL}`);
        const upstreamUrl = new URL(`${AMR_UPSTREAM_URL}/chat/completions`);
        const clientReq = https.request({
          hostname: upstreamUrl.hostname,
          port: upstreamUrl.port || 443,
          path: upstreamUrl.pathname + upstreamUrl.search,
          method: 'POST',
          agent: httpsAgent,
          headers: {
            'Content-Type': 'application/json',
            'Authorization': `Bearer ${upstreamKey}`
          }
        }, upstreamRes => {
          res.writeHead(upstreamRes.statusCode || 200, upstreamRes.headers);
          // A mid-stream upstream error must not leave the client (or the
          // pooled socket) hanging: tear the response down so the socket
          // is released instead of stalling half-open.
          upstreamRes.on('error', err => {
            console.error(`[design-code-proxy] Upstream response error: ${err.message}`);
            res.destroy();
          });
          upstreamRes.pipe(res);
        });

        clientReq.on('error', err => {
          console.error(`[design-code-proxy] Upstream error: ${err.message}. Falling back to local dispatch.`);
          dispatchWithFallback(body, res);
        });

        clientReq.write(bodyText);
        clientReq.end();
      } else {
        // Local smart dispatch
        dispatchWithFallback(body, res);
      }
    });
    return;
  }

  res.writeHead(404, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ error: { message: `Route not found: ${req.method} ${pathname}` } }));
});

const dispatchWithFallback = (body, clientRes) => {
  const reqModel = body.model;
  const resolved = resolveModelConfig(reqModel);
  if (!resolved) {
    clientRes.writeHead(400, { 'Content-Type': 'application/json' });
    clientRes.end(JSON.stringify({
      error: { message: `Unknown model "${reqModel}": not a design-code model (see GET /v1/models)` }
    }));
    return;
  }
  const { modelId, config } = resolved;

  // Prepare messages with design prompt
  const messages = Array.isArray(body.messages) ? [...body.messages] : [];
  const sysIdx = messages.findIndex(m => m.role === 'system');
  if (sysIdx >= 0) {
    const existing = typeof messages[sysIdx].content === 'string'
      ? messages[sysIdx].content
      : JSON.stringify(messages[sysIdx].content);
    messages[sysIdx] = {
      role: 'system',
      content: `${DESIGN_SYSTEM_PROMPT}\n\n${existing}`
    };
  } else {
    messages.unshift({
      role: 'system',
      content: DESIGN_SYSTEM_PROMPT
    });
  }

  const tryTarget = (target, onFail) => {
    const stream = Boolean(body.stream);
    const payload = JSON.stringify({
      ...body,
      model: target.model,
      messages
    });

    console.log(`[design-code-proxy] Trying '${reqModel}' -> 127.0.0.1:${target.port} (${target.model})`);

    const localReq = http.request({
      hostname: '127.0.0.1',
      port: target.port,
      path: '/v1/chat/completions',
      method: 'POST',
      agent: httpAgent,
      headers: {
        'Content-Type': 'application/json',
        'Content-Length': Buffer.byteLength(payload)
      }
    }, localRes => {
      // A backend response error must not hang the client: fall back when
      // nothing was sent yet, otherwise tear the response down so the
      // pooled socket is released instead of stalling half-open.
      localRes.on('error', err => {
        console.warn(`[design-code-proxy] Error reading response from 127.0.0.1:${target.port}: ${err.message}`);
        if (!clientRes.headersSent) onFail(err);
        else clientRes.destroy();
      });

      // Check for quota exhaustion / error codes
      if (localRes.statusCode === 429 || localRes.statusCode >= 500) {
        console.warn(`[design-code-proxy] Target 127.0.0.1:${target.port} returned ${localRes.statusCode}`);
        localRes.resume(); // discard response
        return onFail(new Error(`HTTP ${localRes.statusCode}`));
      }

      // The head is written up front only for streaming. For a buffered
      // (non-stream) response it must wait until the body has been read:
      // the in-body quota check below can still call onFail, and onFail is
      // a no-op once headers are sent — writing the head first made that
      // check hang the request forever instead of falling back (#681).
      // The backend's framing headers describe the backend's body, not
      // the one this proxy sends: the non-stream success path
      // re-serializes the JSON (model rewritten, pretty-printed) and the
      // stream path rewrites each chunk's model, so a forwarded
      // Content-Length truncates the client at the backend's byte count
      // (#684). When the exact body being sent is known, advertise its
      // real length; for streaming, drop the stale framing and let Node
      // chunk the response.
      const writeHead = (sentBody) => {
        if (!clientRes.headersSent) {
          const headers = {
            ...localRes.headers,
            'Access-Control-Allow-Origin': '*'
          };
          delete headers['transfer-encoding'];
          if (sentBody !== undefined) {
            headers['content-length'] = Buffer.byteLength(sentBody);
          } else {
            delete headers['content-length'];
          }
          clientRes.writeHead(localRes.statusCode || 200, headers);
        }
      };

      if (stream) {
        writeHead();
        let buffer = '';
        localRes.on('data', chunk => {
          buffer += chunk.toString('utf-8');
          const lines = buffer.split('\n');
          buffer = lines.pop();
          for (const line of lines) {
            if (line.startsWith('data: ') && line.trim() !== 'data: [DONE]') {
              try {
                const data = JSON.parse(line.slice(6));
                data.model = modelId;
                clientRes.write(`data: ${JSON.stringify(data)}\n\n`);
              } catch (_) {
                clientRes.write(`${line}\n`);
              }
            } else {
              clientRes.write(`${line}\n`);
            }
          }
        });

        localRes.on('end', () => {
          if (buffer) clientRes.write(buffer);
          clientRes.end();
        });
      } else {
        let fullBody = '';
        localRes.on('data', chunk => {
          fullBody += chunk.toString('utf-8');
        });
        localRes.on('end', () => {
          try {
            const json = JSON.parse(fullBody);
            if (json.error && (json.error.code === 429 || json.error.type === 'insufficient_quota')) {
              return onFail(new Error('Quota exceeded in JSON body'));
            }
            json.model = modelId;
            const sentBody = JSON.stringify(json, null, 2);
            writeHead(sentBody);
            clientRes.end(sentBody);
          } catch (_) {
            writeHead(fullBody);
            clientRes.end(fullBody);
          }
        });
      }
    });

    localReq.on('error', err => {
      console.warn(`[design-code-proxy] Error connecting to 127.0.0.1:${target.port}: ${err.message}`);
      onFail(err);
    });

    localReq.write(payload);
    localReq.end();
  };

  // Try primary target, then fallback if needed — skipping any target
  // the live backend catalogs have proven absent (last-known-good view;
  // unvalidated targets are always tried). If every target is proven
  // absent upstream, refuse with 400 rather than silently remapping.
  const allTargets = [config.primary, config.fallback]
    .filter(Boolean)
    .filter((t, i, arr) => i === 0 || t.port !== arr[0].port || t.model !== arr[0].model);
  const liveTargets = allTargets.filter(t => backendTargetKnown(t) !== false);
  if (liveTargets.length === 0) {
    clientRes.writeHead(400, { 'Content-Type': 'application/json' });
    clientRes.end(JSON.stringify({
      error: { message: `Model "${reqModel}" routes only to backend models absent from the live backend catalogs` }
    }));
    return;
  }
  const [firstTarget, secondTarget] = liveTargets;
  tryTarget(firstTarget, err => {
    if (clientRes.headersSent) return;
    if (secondTarget) {
      console.log(`[design-code-proxy] Falling back for '${reqModel}' to 127.0.0.1:${secondTarget.port} (${secondTarget.model})`);
      tryTarget(secondTarget, fallbackErr => {
        if (!clientRes.headersSent) {
          clientRes.writeHead(502, { 'Content-Type': 'application/json' });
          clientRes.end(JSON.stringify({
            error: { message: `All design code backends failed: ${fallbackErr.message}` }
          }));
        }
      });
    } else {
      clientRes.writeHead(502, { 'Content-Type': 'application/json' });
      clientRes.end(JSON.stringify({
        error: { message: `Design code backend failed: ${err.message}` }
      }));
    }
  });
};

server.listen(PORT, HOST, () => {
  console.log(`[design-code-proxy] Listening on http://${HOST}:${PORT}`);
  console.log(`[design-code-proxy] Backends: Antigravity LS (${ANTIGRAVITY_LS_PORT}), Codex (${CODEX_PROXY_PORT}), AMR Upstream (${AMR_UPSTREAM_URL})`);
});
