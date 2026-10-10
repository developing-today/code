#!/usr/bin/env node
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';

const PORT = parseInt(process.env.CLEF_PROXY_PORT || '18742', 10);
const HOST = process.env.CLEF_PROXY_HOST || '127.0.0.1';

const readConfigFileSync = (filename, fallback = '') => {
  try {
    const fullPath = path.join(os.homedir(), '.config', 'cloudflare', filename);
    if (fs.existsSync(fullPath)) {
      return fs.readFileSync(fullPath, 'utf8').trim();
    }
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
      try {
        const body = JSON.parse(bodyText);
        const { accountId, gatewayId, token } = getCredentials(req.headers['authorization']);

        if (!token) {
          res.writeHead(401, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ error: 'Missing Cloudflare AI inference API token' }));
          return;
        }

        const model = resolveModel(body.model);
        const cfUrl = `https://gateway.ai.cloudflare.com/v1/${accountId}/${gatewayId}/workers-ai/run/${model}`;

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

        const cfData = await cfRes.json();

        if (cfRes.ok && cfData.success && cfData.result?.answers) {
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
  console.log(`Clef System One Proxy listening on http://${HOST}:${PORT}`);
  console.log(`Endpoint: http://${HOST}:${PORT}/v1/systemone`);
});
