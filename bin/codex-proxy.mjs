#!/usr/bin/env node
import http from 'node:http';
import { spawn } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';

const PORT = parseInt(process.env.CODEX_PROXY_PORT || '18744', 10);
const HOST = process.env.CODEX_PROXY_HOST || '127.0.0.1';

const MODELS = [
  { id: 'gpt-6.1-sol', name: 'GPT-6.1 Sol (codex)', efforts: ['low', 'medium', 'high', 'xhigh', 'max', 'ultra'], defaultEffort: 'medium' },
  { id: 'gpt-6-astra', name: 'GPT-6 Astra (codex)', efforts: ['low', 'medium', 'high', 'xhigh', 'max', 'ultra'], defaultEffort: 'medium' },
  { id: 'gpt-6-sol', name: 'GPT-6 Sol (codex)', efforts: ['low', 'medium', 'high', 'xhigh', 'max', 'ultra'], defaultEffort: 'medium' },
  { id: 'gpt-6-luna', name: 'GPT-6 Luna (codex)', efforts: ['low', 'medium', 'high', 'xhigh', 'max'], defaultEffort: 'low' },
  { id: 'gpt-5.6-sol', name: 'GPT-5.6 Sol (codex)', efforts: ['low', 'medium', 'high', 'xhigh', 'max', 'ultra'], defaultEffort: 'medium' },
  { id: 'gpt-5.6-terra', name: 'GPT-5.6 Terra (codex)', efforts: ['low', 'medium', 'high', 'xhigh', 'max', 'ultra'], defaultEffort: 'medium' },
  { id: 'gpt-5.6-luna', name: 'GPT-5.6 Luna (codex)', efforts: ['low', 'medium', 'high', 'xhigh', 'max'], defaultEffort: 'low' },
  { id: 'gpt-5.5', name: 'GPT-5.5 (codex)', efforts: ['low', 'medium', 'high', 'xhigh'], defaultEffort: 'low' },
  { id: 'gpt-reserve', name: 'GPT Reserve (codex)', efforts: ['low', 'medium', 'high', 'xhigh', 'max'], defaultEffort: 'low' },
  { id: 'codex-auto-review', name: 'Codex Auto Review', efforts: ['low', 'medium', 'high', 'xhigh', 'max'], defaultEffort: 'medium' }
];

const findCodexBinary = () => {
  const candidates = [
    '/run/current-system/sw/bin/codex',
    `${process.env.HOME}/.local/bin/codex`,
    'codex'
  ];
  for (const c of candidates) {
    if (c === 'codex' || fs.existsSync(c)) {
      return c;
    }
  }
  return 'codex';
};

const CODEX_BIN = findCodexBinary();

const resolveModelAndEffort = (requestedModel, requestedEffort) => {
  let modelStr = (requestedModel || 'gpt-5.6-luna').toLowerCase().trim();
  let effort = requestedEffort ? requestedEffort.toLowerCase().trim() : null;

  // Check if model string ends with effort suffix
  const possibleSuffixes = ['-ultra', '-xhigh', '-high', '-medium', '-low', '-max'];
  for (const suffix of possibleSuffixes) {
    if (modelStr.endsWith(suffix)) {
      effort = suffix.slice(1);
      modelStr = modelStr.slice(0, -suffix.length);
      break;
    }
  }

  // Find base model config
  const modelConfig = MODELS.find(m => m.id === modelStr || m.id.replace(/-/g, '') === modelStr.replace(/-/g, ''))
    || MODELS[6]; // default to gpt-5.6-luna

  // Resolve effort
  if (!effort || !modelConfig.efforts.includes(effort)) {
    effort = modelConfig.defaultEffort;
  }

  return {
    baseModel: modelConfig.id,
    effort,
    fullName: `${modelConfig.id}-${effort}`
  };
};

const formatPrompt = (messages) => {
  if (!Array.isArray(messages) || messages.length === 0) return '';
  if (messages.length === 1 && messages[0].role === 'user') {
    return typeof messages[0].content === 'string'
      ? messages[0].content
      : JSON.stringify(messages[0].content);
  }

  const parts = [];
  const systemMessages = messages.filter(m => m.role === 'system');
  const convoMessages = messages.filter(m => m.role !== 'system');

  if (systemMessages.length > 0) {
    const sysText = systemMessages.map(m =>
      typeof m.content === 'string' ? m.content : JSON.stringify(m.content)
    ).join('\n\n');
    parts.push(`[System Instructions]\n${sysText}`);
  }

  if (convoMessages.length > 0) {
    const convoText = convoMessages.map(m => {
      const role = m.role === 'assistant' ? 'Assistant' : (m.role === 'user' ? 'User' : m.role);
      let text = '';
      if (typeof m.content === 'string') {
        text = m.content;
      } else if (Array.isArray(m.content)) {
        text = m.content.map(c => c.text || JSON.stringify(c)).join('\n');
      } else {
        text = JSON.stringify(m.content);
      }
      return `${role}: ${text}`;
    }).join('\n\n');
    parts.push(convoText);
  }

  return parts.join('\n\n');
};

const server = http.createServer((req, res) => {
  const url = new URL(req.url, `http://${req.headers.host || HOST}`);
  const pathname = url.pathname.replace(/\/+$/, '') || '/';

  // Health & Root
  if (req.method === 'GET' && (pathname === '/health' || pathname === '/')) {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({
      status: 'ok',
      service: 'openai-codex-proxy',
      port: PORT,
      binary: CODEX_BIN,
      defaultModel: 'gpt-5.6-luna-low'
    }, null, 2));
    return;
  }

  // Models catalog
  if (req.method === 'GET' && (pathname === '/v1/models' || pathname === '/models')) {
    const now = Math.floor(Date.now() / 1000);
    const data = [];

    for (const m of MODELS) {
      data.push({
        id: m.id,
        object: 'model',
        created: now,
        owned_by: 'openai-codex'
      });
      for (const eff of m.efforts) {
        data.push({
          id: `${m.id}-${eff}`,
          object: 'model',
          created: now,
          owned_by: 'openai-codex'
        });
      }
    }

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
        res.end(JSON.stringify({ error: { message: 'Request payload too large' } }));
        req.destroy();
      }
    });

    req.on('end', () => {
      let body;
      try {
        body = JSON.parse(bodyText);
      } catch (err) {
        res.writeHead(400, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { message: `Invalid JSON body: ${err.message}` } }));
        return;
      }

      const stream = Boolean(body.stream);
      const reqReasoningEffort = body.reasoning_effort || (body.options && body.options.reasoning_effort);
      const { baseModel, effort, fullName } = resolveModelAndEffort(body.model, reqReasoningEffort);
      const prompt = formatPrompt(body.messages);

      if (!prompt || prompt.trim().length === 0) {
        res.writeHead(400, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { message: 'Messages cannot be empty' } }));
        return;
      }

      const chatId = `chatcmpl-codex-${crypto.randomUUID()}`;
      const created = Math.floor(Date.now() / 1000);

      // Spawn arguments for codex exec
      const args = [
        'exec',
        '--json',
        '--ephemeral',
        '--skip-git-repo-check',
        '--dangerously-bypass-approvals-and-sandbox',
        '--disable', 'code_mode_host',
        '--color', 'never',
        '-m', baseModel,
        '-c', `model_reasoning_effort="${effort}"`,
        '-c', 'model_context_window=1000000',
        '-c', 'model_auto_compact_token_limit=950000',
        '-'
      ];

      console.log(`[codex-proxy] Spawning: ${CODEX_BIN} ${args.join(' ')}`);

      let child;
      try {
        child = spawn(CODEX_BIN, args, {
          cwd: '/tmp',
          env: {
            ...process.env,
            PATH: `/run/current-system/sw/bin:${process.env.PATH || ''}`
          },
          stdio: ['pipe', 'pipe', 'pipe']
        });
      } catch (err) {
        console.error(`[codex-proxy] Failed to spawn codex: ${err.message}`);
        res.writeHead(500, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { message: `Failed to spawn codex: ${err.message}` } }));
        return;
      }

      // Write prompt to stdin and close stdin
      child.stdin.write(prompt);
      child.stdin.end();

      res.on('close', () => {
        if (!res.writableEnded) {
          try { child.kill('SIGTERM'); } catch (_) {}
        }
      });

      if (stream) {
        res.writeHead(200, {
          'Content-Type': 'text/event-stream; charset=utf-8',
          'Cache-Control': 'no-cache, no-transform',
          'Connection': 'keep-alive',
          'X-Accel-Buffering': 'no'
        });
      }

      let lineBuffer = '';
      let fullContent = '';
      let promptTokens = Math.max(1, Math.ceil(prompt.length / 4));
      let completionTokens = 0;

      const processJsonLine = (line) => {
        const trimmed = line.trim();
        if (!trimmed || !trimmed.startsWith('{')) return;

        try {
          const event = JSON.parse(trimmed);

          // Agent message completed
          if (event.type === 'item.completed' && event.item && event.item.type === 'agent_message' && typeof event.item.text === 'string') {
            const text = event.item.text;
            fullContent += text;

            if (stream && !res.writableEnded) {
              const chunk = {
                id: chatId,
                object: 'chat.completion.chunk',
                created,
                model: fullName,
                choices: [{
                  index: 0,
                  delta: { content: text },
                  finish_reason: null
                }]
              };
              res.write(`data: ${JSON.stringify(chunk)}\n\n`);
            }
          }

          // Turn completed with usage metrics
          if (event.type === 'turn.completed' && event.usage) {
            if (typeof event.usage.input_tokens === 'number') {
              promptTokens = event.usage.input_tokens;
            }
            if (typeof event.usage.output_tokens === 'number') {
              completionTokens = event.usage.output_tokens;
            }
          }
        } catch (err) {
          // Non-JSON line or parser error ignored
        }
      };

      child.stdout.on('data', chunk => {
        lineBuffer += chunk.toString('utf-8');
        const lines = lineBuffer.split('\n');
        lineBuffer = lines.pop(); // remainder
        for (const line of lines) {
          processJsonLine(line);
        }
      });

      child.stderr.on('data', chunk => {
        const msg = chunk.toString('utf-8');
        if (msg.trim().length > 0) {
          console.warn(`[codex-proxy stderr] ${msg.trim()}`);
        }
      });

      child.on('close', (code, signal) => {
        if (lineBuffer.trim().length > 0) {
          processJsonLine(lineBuffer);
          lineBuffer = '';
        }

        if (res.writableEnded) return;

        if (stream) {
          if (completionTokens === 0 && fullContent.length > 0) {
            completionTokens = Math.max(1, Math.ceil(fullContent.length / 4));
          }

          const endChunk = {
            id: chatId,
            object: 'chat.completion.chunk',
            created,
            model: fullName,
            choices: [{
              index: 0,
              delta: {},
              finish_reason: 'stop'
            }],
            usage: {
              prompt_tokens: promptTokens,
              completion_tokens: completionTokens,
              total_tokens: promptTokens + completionTokens
            }
          };
          res.write(`data: ${JSON.stringify(endChunk)}\n\n`);
          res.write('data: [DONE]\n\n');
          res.end();
        } else {
          if (completionTokens === 0 && fullContent.length > 0) {
            completionTokens = Math.max(1, Math.ceil(fullContent.length / 4));
          }

          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({
            id: chatId,
            object: 'chat.completion',
            created,
            model: fullName,
            choices: [{
              index: 0,
              message: {
                role: 'assistant',
                content: fullContent
              },
              finish_reason: 'stop'
            }],
            usage: {
              prompt_tokens: promptTokens,
              completion_tokens: completionTokens,
              total_tokens: promptTokens + completionTokens
            }
          }, null, 2));
        }
      });

      child.on('error', err => {
        console.error(`[codex-proxy child error] ${err.message}`);
        if (!res.headersSent) {
          res.writeHead(500, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ error: { message: `Codex execution error: ${err.message}` } }));
        }
      });
    });
    return;
  }

  // Fallthrough 404
  res.writeHead(404, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ error: { message: `Route not found: ${req.method} ${pathname}` } }));
});

server.listen(PORT, HOST, () => {
  console.log(`[codex-proxy] OpenAI-compatible bridge listening on http://${HOST}:${PORT}`);
  console.log(`[codex-proxy] Resolved codex binary: ${CODEX_BIN}`);
});
