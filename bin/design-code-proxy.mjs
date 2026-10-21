#!/usr/bin/env node
import http from 'node:http';
import https from 'node:https';
import { parseProxyPort } from './proxy-port.mjs';

const PORT = parseProxyPort('DESIGN_CODE_PROXY_PORT', 18746);
const HOST = process.env.DESIGN_CODE_PROXY_HOST || '127.0.0.1';

const ANTIGRAVITY_LS_PORT = parseProxyPort('ANTIGRAVITY_LS_PROXY_PORT', 18745);
const CODEX_PROXY_PORT = parseProxyPort('CODEX_PROXY_PORT', 18744);
const AMR_UPSTREAM_URL = process.env.AMR_UPSTREAM_URL || 'https://amr-link.open-design.ai/v1';

const DESIGN_SYSTEM_PROMPT = `You are an elite Code & Systems Design Architect.
Your core expertise is in software architecture, clean code design, UI/UX design engineering, design tokens, component abstractions, domain-driven design, and maintainable systems patterns. Provide elegant, production-grade architectures and idiomatic design implementations.`;

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

  // Fallbacks by keyword
  if (clean.includes('gpt') || clean.includes('codex') || clean.includes('luna') || clean.includes('sol')) {
    return {
      modelId: clean,
      config: {
        primary: { port: CODEX_PROXY_PORT, model: clean },
        fallback: { port: ANTIGRAVITY_LS_PORT, model: 'gemini-3.8-flash' }
      }
    };
  }

  return {
    modelId: clean,
    config: {
      primary: { port: ANTIGRAVITY_LS_PORT, model: 'gemini-3.8-flash' },
      fallback: { port: CODEX_PROXY_PORT, model: 'gpt-5.6-luna' }
    }
  };
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
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({
      status: 'ok',
      service: 'design-code-proxy',
      port: PORT,
      availableModels: MODELS.map(m => m.id)
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
          headers: {
            'Content-Type': 'application/json',
            'Authorization': `Bearer ${upstreamKey}`
          }
        }, upstreamRes => {
          res.writeHead(upstreamRes.statusCode || 200, upstreamRes.headers);
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
  const { modelId, config } = resolveModelConfig(reqModel);

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
      headers: {
        'Content-Type': 'application/json',
        'Content-Length': Buffer.byteLength(payload)
      }
    }, localRes => {
      // Check for quota exhaustion / error codes
      if (localRes.statusCode === 429 || localRes.statusCode >= 500) {
        console.warn(`[design-code-proxy] Target 127.0.0.1:${target.port} returned ${localRes.statusCode}`);
        localRes.resume(); // discard response
        return onFail(new Error(`HTTP ${localRes.statusCode}`));
      }

      clientRes.writeHead(localRes.statusCode || 200, {
        ...localRes.headers,
        'Access-Control-Allow-Origin': '*'
      });

      if (stream) {
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
            clientRes.end(JSON.stringify(json, null, 2));
          } catch (_) {
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

  // Try primary target, then fallback if needed
  tryTarget(config.primary, err => {
    if (clientRes.headersSent) return;
    if (config.fallback && (config.fallback.port !== config.primary.port || config.fallback.model !== config.primary.model)) {
      console.log(`[design-code-proxy] Falling back for '${reqModel}' to 127.0.0.1:${config.fallback.port} (${config.fallback.model})`);
      tryTarget(config.fallback, fallbackErr => {
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
  console.log(`[design-code-proxy] Backends: Antigravity LS (18745), Codex (18744), AMR Upstream (${AMR_UPSTREAM_URL})`);
});
