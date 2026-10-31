#!/usr/bin/env node
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { parseProxyPort } from './proxy-port.mjs';

const PORT = parseProxyPort('CLEF_PROXY_PORT', 18742);
// Test seam (#692): point the gateway base at a stub upstream via
// CLEF_PROXY_GATEWAY_BASE; the Cloudflare gateway stays the default
// and the URL shape after the base is unchanged.
const GATEWAY_BASE = (process.env.CLEF_PROXY_GATEWAY_BASE || 'https://gateway.ai.cloudflare.com/v1').replace(/\/+$/, '');
const HOST = process.env.CLEF_PROXY_HOST || '127.0.0.1';

// Warm connections (#480): upstream calls go through fetch, whose bundled
// undici global dispatcher already pools and reuses keep-alive connections
// to the gateway — there is no per-request handshake to remove. A raw
// https.Agent swap was tried and reverted: https.request bypasses the
// HTTP(S)_PROXY environment (NODE_USE_ENV_PROXY applies to fetch only), so
// on hosts whose egress goes through a proxy the swap broke upstream calls
// outright (TLS to a plain-HTTP proxy port) — a behavior change, not a
// warm-up. Node does not expose undici as an importable module, so the
// dispatcher's pool settings are not configurable from here; the default
// pool is the keep-alive layer for this proxy.

// Publish the bound endpoint so other tools can find this proxy without
// knowing its port in advance: $XDG_STATE_HOME/<name>/endpoint.json
// (default ~/.local/state/<name>/endpoint.json), written on listen,
// removed on exit.
const publishEndpoint = () => {
  try {
    const stateHome =
      process.env.XDG_STATE_HOME || path.join(os.homedir(), '.local', 'state');
    const dir = path.join(stateHome, 'clef-proxy');
    fs.mkdirSync(dir, { recursive: true });
    const file = path.join(dir, 'endpoint.json');
    fs.writeFileSync(
      `${file}.tmp`,
      JSON.stringify({ host: HOST, port: PORT, pid: process.pid, url: `http://${HOST}:${PORT}` }) + '\n',
    );
    fs.renameSync(`${file}.tmp`, file);
    const cleanup = () => {
      try {
        fs.unlinkSync(file);
      } catch {}
    };
    process.on('exit', cleanup);
    process.on('SIGTERM', () => {
      cleanup();
      process.exit(0);
    });
    process.on('SIGINT', () => {
      cleanup();
      process.exit(0);
    });
  } catch {}
};

// Config files were re-read from disk on every request (#480). Cache the
// contents and re-read only when the file's mtime changes, so a rotated
// token is still picked up without a restart but the hot path does a
// single stat instead of up to three full reads.
const configFileCache = new Map();
const readConfigFileSync = (filename, fallback = '') => {
  try {
    const fullPath = path.join(os.homedir(), '.config', 'cloudflare', filename);
    const { mtimeMs } = fs.statSync(fullPath);
    const cached = configFileCache.get(filename);
    if (cached && cached.mtimeMs === mtimeMs) return cached.value;
    const value = fs.readFileSync(fullPath, 'utf8').trim();
    configFileCache.set(filename, { mtimeMs, value });
    return value;
  } catch {}
  return fallback;
};

const getCredentials = (authHeader) => {
  const accountId =
    process.env.CLOUDFLARE_ACCOUNT_ID ||
    readConfigFileSync('account-id', '234b866b99ce2b142d7963d8d139cbfa');

  const gatewayId =
    process.env.CLOUDFLARE_GATEWAY_ID ||
    readConfigFileSync('gateway-id', 'opencode-gateway');

  let token = '';
  if (authHeader && authHeader.startsWith('Bearer ')) {
    token = authHeader.slice(7).trim();
  }
  if (!token) {
    token =
      process.env.CLOUDFLARE_API_TOKEN ||
      readConfigFileSync('ai-inference-token');
  }

  return { accountId, gatewayId, token };
};

const resolveModel = (model) => {
  if (!model || typeof model !== 'string') return '@cf/cloudflare/clef-flash';
  if (model.startsWith('@cf/')) return model;
  if (model === 'clef') return '@cf/cloudflare/clef';
  if (model === 'clef-flash') return '@cf/cloudflare/clef-flash';
  if (model.includes('clef')) return `@cf/cloudflare/${model}`;
  // For generic jev models mapped to clef
  return '@cf/cloudflare/clef-flash';
};

// --- Live model list (2026-10-28 standing rule) ---------------------------
// This proxy is a single-purpose SystemOne endpoint, not a chat router:
// its model universe is the Clef family inside Cloudflare's Workers AI
// catalog. That catalog IS queryable live — GET
// /client/v4/accounts/<id>/ai/models/search with the same inference token
// the data path uses — so GET /v1/models serves the live Clef subset
// (which gained @cf/cloudflare/clef-omni upstream without any code change
// here) with a 5-minute TTL cache and last-known-good fallback to the
// static floor below.
const STATIC_MODELS = ['@cf/cloudflare/clef', '@cf/cloudflare/clef-flash'];
const MODEL_LIST_TTL_MS = 5 * 60 * 1000;
let liveModelCache = { at: 0, models: null };
let liveModelRefresh = null;

const refreshLiveModels = (authHeader) => {
  if (liveModelRefresh) return liveModelRefresh;
  liveModelRefresh = (async () => {
    try {
      const { accountId, token } = getCredentials(authHeader);
      if (!token) throw new Error('no Cloudflare token available');
      const names = [];
      for (let page = 1; page <= 10; page++) {
        const url = `https://api.cloudflare.com/client/v4/accounts/${accountId}/ai/models/search?per_page=100&page=${page}`;
        const res = await fetch(url, {
          headers: { 'Authorization': `Bearer ${token}` },
          signal: AbortSignal.timeout(10000)
        });
        if (!res.ok) throw new Error(`models search HTTP ${res.status}`);
        const data = await res.json();
        const result = Array.isArray(data?.result) ? data.result : [];
        if (result.length === 0) break;
        for (const m of result) {
          if (typeof m?.name === 'string' && m.name.toLowerCase().includes('clef')) names.push(m.name);
        }
        if (result.length < 100) break;
      }
      if (names.length === 0) throw new Error('models search returned no Clef models');
      liveModelCache = { at: Date.now(), models: [...new Set(names)].sort() };
    } catch (err) {
      console.warn(`[clef-proxy] live model refresh failed (${err.message}); serving last-known-good`);
    } finally {
      liveModelRefresh = null;
    }
    return liveModelCache.models || STATIC_MODELS;
  })();
  return liveModelRefresh;
};

const getServedModels = async (authHeader) => {
  if (liveModelCache.models && Date.now() - liveModelCache.at <= MODEL_LIST_TTL_MS) {
    return liveModelCache.models;
  }
  return refreshLiveModels(authHeader);
};

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, `http://${req.headers.host || HOST}`);
  const pathname = url.pathname.replace(/\/+$/, '') || '/';

  if (req.method === 'GET' && (pathname === '/health' || pathname === '/' || pathname === '/v1/systemone')) {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({
      status: 'ok',
      service: 'clef-systemone-proxy',
      port: PORT,
      defaultModel: '@cf/cloudflare/clef-flash',
      endpoint: '/v1/systemone'
    }, null, 2));
    return;
  }

  if (req.method === 'GET' && (pathname === '/v1/models' || pathname === '/models')) {
    getServedModels(req.headers['authorization']).then((models) => {
      const now = Math.floor(Date.now() / 1000);
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({
        object: 'list',
        data: models.map(id => ({ id, object: 'model', created: now, owned_by: 'cloudflare-workers-ai' }))
      }, null, 2));
    });
    return;
  }

  if (req.method === 'POST' && (pathname === '/v1/systemone' || pathname === '/systemone' || pathname === '/')) {
    let bodyText = '';
    req.on('data', chunk => {
      bodyText += chunk;
      if (bodyText.length > 5 * 1024 * 1024) {
        res.writeHead(413, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: 'Request body too large' }));
        req.destroy();
      }
    });

    req.on('end', async () => {
      // Parse the client body on its own: a malformed request is a 400,
      // not a 500 from the shared upstream catch below (sibling proxies
      // all parse separately — #690).
      let body;
      try {
        body = JSON.parse(bodyText);
      } catch (err) {
        res.writeHead(400, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: `Invalid JSON body: ${err.message}` }));
        return;
      }
      if (!body || typeof body !== 'object') {
        res.writeHead(400, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: 'Invalid JSON body: expected an object' }));
        return;
      }

      try {
        const { accountId, gatewayId, token } = getCredentials(req.headers['authorization']);

        if (!token) {
          res.writeHead(401, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ error: 'Missing Cloudflare AI inference API token' }));
          return;
        }

        const model = resolveModel(body.model);
        const cfUrl = `${GATEWAY_BASE}/${accountId}/${gatewayId}/workers-ai/run/${model}`;

        const payload = {
          state: body.state || {},
          questions: body.questions || {}
        };

        const cfRes = await fetch(cfUrl, {
          method: 'POST',
          headers: {
            'Authorization': `Bearer ${token}`,
            'Content-Type': 'application/json'
          },
          body: JSON.stringify(payload)
        });

        // The gateway can answer non-JSON (e.g. an HTML error page) or a
        // JSON null/scalar: neither is a client parse error, so keep the
        // upstream status and report it as an upstream failure (#690).
        let cfData = null;
        try {
          cfData = await cfRes.json();
        } catch {}

        if (cfRes.ok && cfData?.success && cfData.result?.answers) {
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ answers: cfData.result.answers }));
        } else {
          res.writeHead(cfRes.status >= 400 ? cfRes.status : 502, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({
            error: 'Cloudflare Workers AI Clef execution failed',
            details: cfData
          }));
        }
      } catch (err) {
        res.writeHead(500, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: err.message || 'Internal proxy error' }));
      }
    });
    return;
  }

  res.writeHead(404, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ error: 'Not Found', path: pathname }));
});

server.listen(PORT, HOST, () => {
  publishEndpoint();
  console.log(`Clef System One Proxy listening on http://${HOST}:${PORT}`);
  console.log(`Endpoint: http://${HOST}:${PORT}/v1/systemone`);
});
