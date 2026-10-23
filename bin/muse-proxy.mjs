#!/usr/bin/env node
// muse-proxy — OpenAI-compatible bridge to the Meta Muse Code CLI.
//
// Mirrors bin/codex-proxy.mjs in developing-today/code: one `muse exec --json`
// subprocess per chat-completions request, prompt delivered via --prompt-file,
// streamed back out of the CLI's JSONL event stream.
//
// Auth is the CLI's own OAuth login (~/.config/muse/auth.json). META_API_KEY
// must NOT be set for the child: the CLI binary states it takes priority over
// the account login, and the Model API key is a separate pay-as-you-go wallet
// (inference there returns 402 until funded). The child is therefore spawned
// with META_API_KEY explicitly deleted and with the sandbox's unauthenticated
// egress proxy + CA bundle, the only network path the CLI can reach here.
import http from 'node:http';
import { spawn } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseProxyPort } from './proxy-port.mjs';

// Default port 18747: 18746 is claimed by design-code-proxy
// (DESIGN_CODE_PROXY_PORT) in this repo's config, so muse-proxy takes the
// next port in the family range (cf. clef 18742, agy 18743, codex 18744,
// antigravity-ls 18745, design-code 18746).
const PORT = parseProxyPort('MUSE_PROXY_PORT', 18747);
const HOST = process.env.MUSE_PROXY_HOST || '127.0.0.1';

const EGRESS_PROXY = 'http://hatch-egress-proxy:3128';
const EGRESS_CA = '/run/hatch/egress-tls/ca-bundle.pem';

const PROXY_DIR = path.dirname(fileURLToPath(import.meta.url));
const WORKDIR = process.env.MUSE_PROXY_WORKDIR || path.join(PROXY_DIR, 'work');
fs.mkdirSync(WORKDIR, { recursive: true });

const EFFORTS = ['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra'];
const MODELS = [
  { id: 'muse-spark-1.3', name: 'Muse Spark 1.3 (muse)', efforts: EFFORTS, defaultEffort: 'high' },
  { id: 'muse-wonder-1.0', name: 'Muse Wonder 1.0 (muse)', efforts: EFFORTS, defaultEffort: 'high' }
];

const findMuseBinary = () => {
  const candidates = [
    `${process.env.HOME}/.local/bin/muse`,
    '/run/current-system/sw/bin/muse',
    'muse'
  ];
  for (const c of candidates) {
    if (c === 'muse' || fs.existsSync(c)) {
      return c;
    }
  }
  return 'muse';
};

const MUSE_BIN = findMuseBinary();

const resolveModelAndEffort = (requestedModel, requestedEffort) => {
  let rawModel = (requestedModel || 'muse-spark-1.3').trim();
  // Strip any provider prefix before '/' (e.g. 'muse/', 'meta/', 'muse-code/')
  let modelStr = rawModel.replace(/^.*\//, '').toLowerCase().trim();
  let effort = requestedEffort ? requestedEffort.toLowerCase().trim() : null;

  // Check if model string ends with effort suffix (longest first)
  const possibleSuffixes = ['-minimal', '-medium', '-xhigh', '-ultra', '-high', '-none', '-low', '-max'];
  for (const suffix of possibleSuffixes) {
    if (modelStr.endsWith(suffix)) {
      effort = suffix.slice(1);
      modelStr = modelStr.slice(0, -suffix.length);
      break;
    }
  }

  // Bare aliases
  if (modelStr === 'spark' || modelStr === 'muse-spark') modelStr = 'muse-spark-1.3';
  if (modelStr === 'wonder' || modelStr === 'muse-wonder') modelStr = 'muse-wonder-1.0';

  // Find base model config
  const modelConfig = MODELS.find(m => m.id === modelStr || m.id.replace(/-/g, '') === modelStr.replace(/-/g, ''))
    || MODELS[0]; // default to muse-spark-1.3

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
  if (messages.length === 1 && messages[0].role === 'user' && typeof messages[0].content === 'string') {
    return messages[0].content;
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
      let role = m.role === 'assistant' ? 'Assistant' : (m.role === 'user' ? 'User' : m.role);
      let text = '';
      if (typeof m.content === 'string') {
        text = m.content;
      } else if (Array.isArray(m.content)) {
        text = m.content.map(c => c.text || JSON.stringify(c)).join('\n');
      } else if (m.content) {
        text = JSON.stringify(m.content);
      }

      if (m.tool_calls && Array.isArray(m.tool_calls)) {
        const tcStr = m.tool_calls.map(tc => `[Tool Call: ${tc.function?.name || 'function'}(${tc.function?.arguments || ''})]`).join('\n');
        text = text ? `${text}\n${tcStr}` : tcStr;
      }
      if (m.role === 'tool') {
        role = `Tool Result (${m.tool_call_id || 'result'})`;
      }

      return `${role}: ${text}`;
    }).join('\n\n');
    parts.push(convoText);
  }

  return parts.join('\n\n');
};

// Child env: inherit, but strip META_API_KEY (it outranks the OAuth login)
// and pin the egress proxy + CA bundle the CLI needs to reach Meta.
const childEnv = () => {
  const env = { ...process.env };
  delete env.META_API_KEY;
  env.HTTPS_PROXY = EGRESS_PROXY;
  env.HTTP_PROXY = EGRESS_PROXY;
  env.https_proxy = EGRESS_PROXY;
  env.http_proxy = EGRESS_PROXY;
  env.SSL_CERT_FILE = EGRESS_CA;
  return env;
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

  // Health & Root
  if (req.method === 'GET' && (pathname === '/health' || pathname === '/')) {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({
      status: 'ok',
      service: 'meta-muse-proxy',
      port: PORT,
      binary: MUSE_BIN,
      defaultModel: 'muse-spark-1.3-high',
      endpoint: '/v1/chat/completions'
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
        owned_by: 'meta-muse'
      });
      for (const eff of m.efforts) {
        data.push({
          id: `${m.id}-${eff}`,
          object: 'model',
          created: now,
          owned_by: 'meta-muse'
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

      const chatId = `chatcmpl-muse-${crypto.randomUUID()}`;
      const created = Math.floor(Date.now() / 1000);

      // Prompt goes in via --prompt-file (no argv length/escaping concerns)
      const promptFile = path.join(WORKDIR, `prompt-${crypto.randomUUID()}.txt`);
      fs.writeFileSync(promptFile, prompt, 'utf-8');

      // Spawn arguments for muse exec
      const args = [
        'exec',
        '--json',
        '--prompt-file', promptFile,
        '--model', baseModel,
        '--reasoning-effort', effort,
        '--disable-reminders',
        '--user-input-auto-resolve',
        '--approval-mode', 'never'
      ];

      console.log(`[muse-proxy] Spawning: ${MUSE_BIN} ${args.join(' ')}`);

      let child;
      try {
        child = spawn(MUSE_BIN, args, {
          cwd: WORKDIR,
          env: childEnv(),
          stdio: ['ignore', 'pipe', 'pipe']
        });
      } catch (err) {
        console.error(`[muse-proxy] Failed to spawn muse: ${err.message}`);
        try { fs.unlinkSync(promptFile); } catch (_) {}
        res.writeHead(500, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { message: `Failed to spawn muse: ${err.message}` } }));
        return;
      }

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
      let terminalState = null;
      let terminalText = null;
      let stderrTail = '';

      const processJsonLine = (line) => {
        const trimmed = line.trim();
        if (!trimmed || !trimmed.startsWith('{')) return;

        try {
          const event = JSON.parse(trimmed);
          const payload = event.payload || {};

          // Assistant text delta (streams token chunks)
          if (event.payload_type === 'run.output.delta' && typeof payload.text === 'string') {
            const text = payload.text;
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

          // Real token usage from the run
          if (event.payload_type === 'run.usage' && Array.isArray(payload.models)) {
            let inTok = 0;
            let outTok = 0;
            for (const m of payload.models) {
              if (typeof m.input_tokens === 'number') inTok += m.input_tokens;
              if (typeof m.output_tokens === 'number') outTok += m.output_tokens;
            }
            if (inTok > 0) promptTokens = inTok;
            if (outTok > 0) completionTokens = outTok;
          }

          // Terminal event carries the authoritative final text
          if (typeof event.payload_type === 'string' && event.payload_type.startsWith('run.terminal.')) {
            terminalState = payload.terminal || event.payload_type.slice('run.terminal.'.length);
            if (typeof payload.text === 'string') terminalText = payload.text;
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
        stderrTail = (stderrTail + msg).slice(-2000);
        if (msg.trim().length > 0) {
          console.warn(`[muse-proxy stderr] ${msg.trim()}`);
        }
      });

      child.on('close', (code, signal) => {
        if (lineBuffer.trim().length > 0) {
          processJsonLine(lineBuffer);
          lineBuffer = '';
        }
        try { fs.unlinkSync(promptFile); } catch (_) {}

        if (res.writableEnded) return;

        // Reconcile the streamed deltas with the terminal event's
        // authoritative final text (#484). In stream mode the deltas are
        // already on the wire, so the only recoverable divergence is a
        // missing tail: when the terminal text extends the streamed text,
        // the suffix is emitted as one last content chunk before the
        // finish chunk below, so streamed and non-streamed clients end up
        // with identical content. Any other divergence (the terminal text
        // is not an extension of what was streamed) cannot be retracted
        // from a stream; it is logged explicitly instead of silently
        // truncating, and the non-streamed response still uses the
        // authoritative text when it is at least as long.
        let unstreamedSuffix = '';
        if (terminalText !== null) {
          if (terminalText.startsWith(fullContent)) {
            unstreamedSuffix = terminalText.slice(fullContent.length);
          } else if (terminalText.length >= fullContent.length) {
            console.warn('[muse-proxy] terminal text diverges from streamed deltas; streamed response cannot be corrected retroactively');
          }
          if (terminalText.length >= fullContent.length) {
            fullContent = terminalText;
          }
        }

        // Failure is judged by the run's authoritative terminal state when
        // one was emitted, otherwise by the exit code (#672). Partial
        // content must not mask a failure: a run that failed midway used to
        // be reported as a successful 'stop' carrying the truncated text,
        // because both clauses of the old predicate required empty content.
        const failed = terminalState ? terminalState !== 'completed' : code !== 0;
        if (failed) {
          const stderrLine = stderrTail.trim().split('\n').pop();
          const fallback = terminalState && terminalState !== 'completed'
            ? `muse run terminated with state '${terminalState}'`
            : signal
              ? `muse exec terminated by signal ${signal}`
              : `muse exec exited with code ${code}`;
          const detail = (stderrLine || fallback).trim();
          const errorBody = { error: { message: `Muse execution failed: ${detail}` } };
          if (stream) {
            // The 200 + text/event-stream headers are already on the wire,
            // so a bare JSON body would be invisible to an SSE client
            // parsing `data:` lines — it would see an empty, successful
            // stream (#670). Frame the error as an SSE event and close
            // the stream with [DONE] instead.
            if (!res.writableEnded) {
              res.write(`data: ${JSON.stringify(errorBody)}\n\n`);
              res.write('data: [DONE]\n\n');
            }
            res.end();
            return;
          }
          if (!res.headersSent) {
            res.writeHead(500, { 'Content-Type': 'application/json' });
          }
          res.end(JSON.stringify(errorBody));
          return;
        }

        if (completionTokens === 0 && fullContent.length > 0) {
          completionTokens = Math.max(1, Math.ceil(fullContent.length / 4));
        }

        if (stream) {
          if (unstreamedSuffix.length > 0) {
            const tailChunk = {
              id: chatId,
              object: 'chat.completion.chunk',
              created,
              model: fullName,
              choices: [{
                index: 0,
                delta: { content: unstreamedSuffix },
                finish_reason: null
              }]
            };
            res.write(`data: ${JSON.stringify(tailChunk)}\n\n`);
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
        console.error(`[muse-proxy child error] ${err.message}`);
        try { fs.unlinkSync(promptFile); } catch (_) {}
        if (!res.headersSent) {
          res.writeHead(500, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ error: { message: `Muse execution error: ${err.message}` } }));
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
  console.log(`[muse-proxy] OpenAI-compatible bridge listening on http://${HOST}:${PORT}`);
  console.log(`[muse-proxy] Resolved muse binary: ${MUSE_BIN}`);
  console.log(`[muse-proxy] Workdir: ${WORKDIR}`);
});
