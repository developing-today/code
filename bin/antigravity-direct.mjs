#!/usr/bin/env node
// antigravity-direct.mjs — a direct Google Cloud Code (v1internal) door for
// the Antigravity consumer account, in the style of muse-proxy's
// "muse-direct" mode: skip the intermediary process, call the upstream the
// same way the first-party client does.
//
// What this replaces: antigravity-ls-proxy.mjs (spawns Google's
// language_server) and agy-proxy.mjs (spawns `agy` CLI workers). Both exist
// only to borrow the CLI's auth + request shape. This proxy reproduces that
// wire behavior directly, verified against a live capture of the real CLI
// (2026-10-30, see NOTES in the build dir):
//
//   POST https://daily-cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse
//   Headers: Authorization: Bearer <OAuth access token>, Content-Type:
//     application/json, Accept-Encoding: gzip, and the CLI's own
//     User-Agent: antigravity/cli/<ver> (aidev_client; os_type=...; ...)
//   Body: { project: "aicode-consumers", requestId: "agent/<uuid>/<ms>/<uuid>/1",
//     model: "<cli model id, e.g. gemini-3.8-flash-high>",
//     userAgent: "antigravity", requestType: "agent",
//     request: { contents, systemInstruction?, tools?, toolConfig?,
//       generationConfig: { maxOutputTokens: 65536,
//         thinkingConfig: { includeThoughts: true, thinkingLevel: "HIGH" } },
//       sessionId: "<negative int64 as string>" } }
//
// Auth is the CLI's auth, unchanged:
//   - The live credential is the OAuth token set the CLI keeps in the OS
//     keyring (Secret Service) under service="gemini", username="antigravity"
//     (shape: { token: { access_token, refresh_token, expiry, token_type },
//     auth_method: "consumer", id_token }). Read via `secret-tool lookup`.
//     The ~/.gemini/oauth_creds.json file is only the CLI's fallback store —
//     on this machine it is stale and is NOT what the CLI runs on, so this
//     proxy does not use it either.
//   - Refresh is the CLI's refresh: POST https://oauth2.googleapis.com/token
//     with grant_type=refresh_token, the refresh token above, and the CLI's
//     own embedded OAuth client (id 1071006060591-…apps.googleusercontent.com
//     — a public identifier). Google does not rotate the refresh token for
//     this client (verified: refresh responses carry no new refresh_token),
//     so refreshing here never invalidates the CLI's own credential. The
//     client secret is extracted from the agy binary at runtime (the first
//     35 chars of its embedded GOCSPX blob — the pairing was verified live)
//     and is never written to disk or logged by this proxy.
//   - The proxy never writes tokens anywhere: no keyring writes, no files.
//     It caches the access token in memory only.
//
// Interface mirrors agy-proxy.mjs: OpenAI-compatible POST
// /v1/chat/completions (stream + non-stream) and GET /v1/models, base model
// ids (gemini-4-argon, gemini-3.8-flash, …) with reasoning_effort or a
// -low/-medium/-high suffix selecting the CLI's thinking level.

import http from 'node:http';
import https from 'node:https';
import { execFile } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import crypto from 'node:crypto';
import zlib from 'node:zlib';

const HOST = process.env.ANTIGRAVITY_DIRECT_HOST || '127.0.0.1';
const PORT = Number(process.env.ANTIGRAVITY_DIRECT_PORT || 18751);
const UPSTREAM = (process.env.ANTIGRAVITY_DIRECT_UPSTREAM || 'https://daily-cloudcode-pa.googleapis.com').replace(/\/$/, '');
const TIMEOUT_MS = Number(process.env.ANTIGRAVITY_DIRECT_TIMEOUT_MS || 300000);
const PROJECT = 'aicode-consumers';

// The CLI's OAuth client id (public identifier, appears in its sign-in URLs).
const OAUTH_CLIENT_ID = '1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com';
const FALLBACK_UA = 'antigravity/cli/1.4.11 (aidev_client; os_type=linux; arch=amd64; cl=1010154931; auth_method=consumer)';

// Catalog mirrors agy-proxy.mjs FALLBACK_MODELS (pulled from the live CLI).
const FALLBACK_MODELS = [
  { id: 'gemini-4-argon', name: 'Gemini 4 Argon', efforts: ['low', 'medium', 'high'], defaultEffort: 'medium' },
  { id: 'gemini-3.8-flash', name: 'Gemini 3.8 Flash', efforts: ['low', 'medium', 'high'], defaultEffort: 'medium' },
  { id: 'claude-sonnet-5-5', name: 'Claude Sonnet 5.5', efforts: ['low', 'medium', 'high'], defaultEffort: 'medium' },
  { id: 'claude-opus-5-5', name: 'Claude Opus 5.5', efforts: ['low', 'medium', 'high'], defaultEffort: 'medium' },
  { id: 'gpt-oss-120b', name: 'GPT-OSS 120B', efforts: ['medium'], defaultEffort: 'medium' }
];
let modelCatalog = FALLBACK_MODELS;

const log = (...a) => console.error(new Date().toISOString(), ...a);

// ---------------------------------------------------------------------------
// agy binary introspection: User-Agent + OAuth client secret (runtime only)
// ---------------------------------------------------------------------------

const findAgyBinary = () => {
  const candidates = [
    process.env.AGY_BIN,
    path.join(os.homedir(), '.gemini', 'bin', 'agy'),
    '/run/current-system/sw/bin/agy'
  ].filter(Boolean);
  for (const c of candidates) { try { fs.accessSync(c, fs.constants.X_OK); return c; } catch {} }
  return null;
};

let _binaryIntel = null;
const binaryIntel = () => {
  if (_binaryIntel) return _binaryIntel;
  const bin = findAgyBinary();
  let ua = FALLBACK_UA;
  let clientSecret = null;
  if (bin) {
    // Chunked scan so we never hold the ~210MB binary in memory at once.
    const fd = fs.openSync(bin, 'r');
    const chunk = Buffer.alloc(1 << 20);
    let carry = '';
    let pos = 0;
    let n;
    while ((n = fs.readSync(fd, chunk, 0, chunk.length, pos)) > 0) {
      pos += n;
      const text = carry + chunk.toString('latin1', 0, n);
      if (!clientSecret) {
        const m = text.match(/GOCSPX-[A-Za-z0-9_-]{10,}/);
        // The binary embeds two client secrets concatenated (70 chars);
        // the first 35 chars pair with OAUTH_CLIENT_ID (verified live).
        if (m) clientSecret = m[0].slice(0, 35);
      }
      if (ua === FALLBACK_UA) {
        const m = text.match(/antigravity\/cli\/[0-9.]+ \(aidev_client;[^)]*\)/);
        if (m) ua = m[0];
      }
      if (clientSecret && ua !== FALLBACK_UA) break;
      carry = text.slice(-256);
    }
    fs.closeSync(fd);
  }
  _binaryIntel = { ua, clientSecret };
  return _binaryIntel;
};

// ---------------------------------------------------------------------------
// Credential handling: keyring read (secret-tool) + OAuth refresh
// ---------------------------------------------------------------------------

let _secretTool = null;
const findSecretTool = () => {
  if (_secretTool) return _secretTool;
  const candidates = [];
  if (process.env.SECRET_TOOL_PATH) candidates.push(process.env.SECRET_TOOL_PATH);
  for (const dir of (process.env.PATH || '').split(':')) if (dir) candidates.push(path.join(dir, 'secret-tool'));
  try {
    const stores = fs.readdirSync('/nix/store').filter(d => d.includes('libsecret')).sort();
    for (const s of stores) candidates.push(path.join('/nix/store', s, 'bin', 'secret-tool'));
  } catch {}
  for (const c of candidates) { try { fs.accessSync(c, fs.constants.X_OK); _secretTool = c; return c; } catch {} }
  throw new Error('secret-tool not found (set SECRET_TOOL_PATH)');
};

const readKeyringCreds = () => new Promise((resolve, reject) => {
  const env = { ...process.env };
  if (!env.DBUS_SESSION_BUS_ADDRESS) {
    env.DBUS_SESSION_BUS_ADDRESS = 'unix:path=/run/user/' + (typeof process.getuid === 'function' ? process.getuid() : 1000) + '/bus';
  }
  execFile(findSecretTool(), ['lookup', 'service', 'gemini', 'username', 'antigravity'],
    { env, timeout: 15000, maxBuffer: 1 << 20 }, (err, stdout) => {
      if (err) return reject(new Error('keyring lookup failed: ' + err.message));
      try {
        const creds = JSON.parse(stdout);
        if (!creds.token || !creds.token.refresh_token) return reject(new Error('keyring credential has unexpected shape'));
        resolve(creds);
      } catch (e) { reject(new Error('keyring credential is not JSON: ' + e.message)); }
    });
});

// ISO timestamps from the keyring carry nanosecond fractions; trim for Date.
const parseExpiry = (s) => {
  if (!s) return 0;
  const t = Date.parse(String(s).replace(/(\.\d{3})\d+/, '$1'));
  return Number.isNaN(t) ? 0 : t;
};

const postForm = (url, params) => new Promise((resolve, reject) => {
  const body = new URLSearchParams(params).toString();
  const req = https.request(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded', 'Content-Length': Buffer.byteLength(body) }
  }, (res) => {
    const chunks = [];
    res.on('data', c => chunks.push(c));
    res.on('end', () => {
      const text = Buffer.concat(chunks).toString('utf8');
      if (res.statusCode !== 200) return reject(new Error('token endpoint HTTP ' + res.statusCode + ': ' + text.slice(0, 200)));
      try { resolve(JSON.parse(text)); } catch (e) { reject(e); }
    });
  });
  req.on('error', reject);
  req.end(body);
});

let _token = null; // { accessToken, expiryMs, refreshToken }
let _tokenPromise = null;

const getAccessToken = (forceRefresh = false) => {
  if (!forceRefresh && _token && _token.expiryMs - Date.now() > 60000) return Promise.resolve(_token.accessToken);
  if (_tokenPromise) return _tokenPromise;
  _tokenPromise = (async () => {
    try {
      const creds = await readKeyringCreds();
      const kt = creds.token;
      const keyringExpiry = parseExpiry(kt.expiry);
      if (!forceRefresh && kt.access_token && keyringExpiry - Date.now() > 60000) {
        _token = { accessToken: kt.access_token, expiryMs: keyringExpiry, refreshToken: kt.refresh_token };
        return _token.accessToken;
      }
      const { clientSecret } = binaryIntel();
      if (!clientSecret) throw new Error('could not extract OAuth client secret from agy binary');
      const resp = await postForm('https://oauth2.googleapis.com/token', {
        client_id: OAUTH_CLIENT_ID,
        client_secret: clientSecret,
        refresh_token: kt.refresh_token,
        grant_type: 'refresh_token'
      });
      _token = {
        accessToken: resp.access_token,
        expiryMs: Date.now() + (resp.expires_in || 3600) * 1000,
        refreshToken: kt.refresh_token
      };
      log('refreshed access token via oauth2.googleapis.com (expires_in=' + resp.expires_in + ')');
      return _token.accessToken;
    } finally {
      _tokenPromise = null;
    }
  })();
  return _tokenPromise;
};

// ---------------------------------------------------------------------------
// Model catalog: try the CLI's own fetchAvailableModels, fall back to static
// ---------------------------------------------------------------------------

const refreshCatalog = async () => {
  try {
    const token = await getAccessToken();
    const data = await upstreamJson('/v1internal:fetchAvailableModels', token, { project: PROJECT });
    const models = data && (data.models || (data.response && data.response.models));
    if (Array.isArray(models) && models.length) {
      const byBase = new Map();
      for (const m of models) {
        const id = m.name || m.id || '';
        const mm = String(id).match(/^(.*)-(low|medium|high)$/);
        if (!mm) continue;
        if (!byBase.has(mm[1])) byBase.set(mm[1], { id: mm[1], name: mm[1], efforts: [], defaultEffort: 'medium' });
        byBase.get(mm[1]).efforts.push(mm[2]);
      }
      if (byBase.size) {
        modelCatalog = [...byBase.values()];
        log('catalog refreshed from fetchAvailableModels:', modelCatalog.map(m => m.id).join(', '));
      }
    }
  } catch (e) {
    log('catalog pull failed, keeping fallback list:', e.message);
  }
};

const EFFORTS = ['low', 'medium', 'high'];
const resolveModelAndEffort = (requestedModel, requestedEffort) => {
  let model = String(requestedModel || '').trim();
  let effort = requestedEffort ? String(requestedEffort).toLowerCase() : null;
  const suffix = model.match(/^(.*)-(low|medium|high|xhigh|max)$/);
  if (suffix) {
    model = suffix[1];
    if (!effort) effort = suffix[2];
  }
  const cfg = modelCatalog.find(m => m.id === model);
  if (effort === 'xhigh' || effort === 'max') effort = 'high'; // CLI exposes up to high in model ids
  if (!effort || !EFFORTS.includes(effort)) effort = cfg ? cfg.defaultEffort : 'medium';
  return { base: model, effort, fullName: model + '-' + effort };
};

// ---------------------------------------------------------------------------
// OpenAI -> Cloud Code v1internal translation
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Tool schema conversion: client JSON Schema -> the v1internal proto Schema
// (google.cloud.aiplatform.master.Schema). The proto parser rejects any
// field name it does not know with a hard 400 ("Unknown name ... Cannot
// find field"), and client tool schemas are full JSON Schema (usually
// zod/MCP-generated), so they must be reduced to the proto's vocabulary
// before going upstream. The rule set follows the fork's established
// Gemini schema normalizer (opencode packages/ai/src/protocols/utils/
// gemini-json-schema.ts + tool-schema.ts: rewrite only what the upstream
// rejects, filter `required` to declared properties, translate exclusive
// bounds, cut unsafe $ref loops, object root), tightened to the field
// allowlist probed against this endpoint 2026-10-30 (code#2825):
//   accepted: type format title description nullable default enum example
//     items prefixItems minItems maxItems minProperties maxProperties
//     properties propertyOrdering required minLength maxLength pattern
//     minimum maximum anyOf oneOf allOf not
//   rejected (dropped or translated below): $schema $id $ref $defs const
//     exclusiveMinimum exclusiveMaximum multipleOf additionalProperties
//     additionalItems unevaluatedItems unevaluatedProperties contains
//     minContains uniqueItems dependentRequired dependentSchemas
//     patternProperties if then else readOnly writeOnly deprecated examples
// Structural rejects probed the same day: array `type` (collapse it;
// "null" becomes nullable), tuple `items` (becomes prefixItems), boolean
// schemas (become {}), `required` naming an undeclared property, and
// `properties` on a node whose type is not OBJECT.
// ---------------------------------------------------------------------------

const SCHEMA_FIELDS = new Set([
  'type', 'format', 'title', 'description', 'nullable', 'default', 'enum',
  'example', 'items', 'prefixItems', 'minItems', 'maxItems', 'minProperties',
  'maxProperties', 'properties', 'propertyOrdering', 'required', 'minLength',
  'maxLength', 'pattern', 'minimum', 'maximum', 'anyOf', 'oneOf', 'allOf',
  'not'
]);
// Keys whose values are data, not schemas — never recurse into them.
const SCHEMA_VALUE_KEYS = new Set(['enum', 'default', 'example']);
// Keys whose values are maps of subschemas.
const SCHEMA_MAP_KEYS = new Set(['properties', 'patternProperties', '$defs', 'definitions', 'dependentSchemas', 'dependencies']);

const isSchemaRecord = (v) => !!v && typeof v === 'object' && !Array.isArray(v);

// Inline local $refs (#, #/$defs/<name>, #/definitions/<name>, any local
// JSON pointer). The proto has no $ref, so every reference must be inlined
// or cut. A ref whose target is already being expanded on this path (a
// loop), an unresolvable ref, or a non-local ref is cut to an
// unconstrained schema ({}) — the same cut gemini-json-schema.ts applies
// to loops Gemini cannot represent. Keys sibling to $ref win over the
// referenced schema (JSON Schema 2020-12 semantics).
const resolveSchemaRefs = (root) => {
  const lookup = (ref) => {
    if (ref === '#') return root;
    if (!ref.startsWith('#/')) return undefined;
    let cur = root;
    for (const rawSeg of ref.slice(2).split('/')) {
      const seg = rawSeg.replace(/~1/g, '/').replace(/~0/g, '~');
      if ((!isSchemaRecord(cur) && !Array.isArray(cur)) || cur[seg] === undefined) return undefined;
      cur = cur[seg];
    }
    return cur;
  };
  const walkSchema = (node, chain) => {
    if (Array.isArray(node)) return node.map(v => walkSchema(v, chain));
    if (!isSchemaRecord(node)) return node;
    if (typeof node.$ref === 'string') {
      const { $ref, ...siblings } = node;
      const target = $ref.startsWith('#') ? lookup($ref) : undefined;
      const merged = (target === undefined || chain.includes($ref) || chain.length >= 10)
        ? {} : { ...walkSchema(target, [...chain, $ref]) };
      for (const [k, v] of Object.entries(siblings)) merged[k] = walkSchema(v, chain);
      return merged;
    }
    const out = {};
    for (const [k, v] of Object.entries(node)) {
      if (SCHEMA_VALUE_KEYS.has(k)) { out[k] = v; continue; }
      if (SCHEMA_MAP_KEYS.has(k) && isSchemaRecord(v)) {
        const sub = {};
        for (const [name, s] of Object.entries(v)) sub[name] = walkSchema(s, chain);
        out[k] = sub;
        continue;
      }
      out[k] = walkSchema(v, chain);
    }
    return out;
  };
  return walkSchema(root, []);
};

const convertSchemaNode = (node) => {
  // Boolean schemas and stray values become an unconstrained object schema.
  if (!isSchemaRecord(node)) return {};
  const out = {};
  // Exclusive bounds: the proto has none — fold them into the inclusive
  // bound. A numeric (draft-06+) exclusive value becomes the bound when no
  // inclusive one is present; a boolean (draft-04) flag simply drops,
  // keeping its sibling bound.
  let minimum = node.minimum;
  let maximum = node.maximum;
  if (typeof node.exclusiveMinimum === 'number' && minimum === undefined) minimum = node.exclusiveMinimum;
  if (typeof node.exclusiveMaximum === 'number' && maximum === undefined) maximum = node.exclusiveMaximum;
  // Tuple `items` becomes prefixItems (proto `items` is a single schema).
  // Upstream validation requires `items` to be present whenever
  // prefixItems is ("properties[t].items: missing field", probed
  // 2026-10-30), so a converted tuple also gets an unconstrained items —
  // which matches draft-07 semantics (elements past the tuple are free
  // unless additionalItems says otherwise; additionalItems is dropped).
  let items = node.items;
  let prefixItems = node.prefixItems;
  if (Array.isArray(items)) {
    if (items.length && prefixItems === undefined) prefixItems = items;
    items = undefined;
  }
  if (prefixItems !== undefined && items === undefined) items = {};
  for (const [k, v] of Object.entries(node)) {
    if (!SCHEMA_FIELDS.has(k)) continue; // unknown to the proto -> drop
    if (SCHEMA_VALUE_KEYS.has(k)) { out[k] = v; continue; }
    if (k === 'properties') {
      if (isSchemaRecord(v)) {
        const props = {};
        for (const [name, sub] of Object.entries(v)) props[name] = convertSchemaNode(sub);
        out.properties = props;
      }
      continue;
    }
    if (k === 'anyOf' || k === 'oneOf' || k === 'allOf') {
      if (Array.isArray(v)) out[k] = v.map(convertSchemaNode);
      continue;
    }
    if (k === 'not') { out.not = convertSchemaNode(v); continue; }
    if (k === 'type' || k === 'required' || k === 'minimum' || k === 'maximum' || k === 'items' || k === 'prefixItems') continue; // handled outside the loop
    out[k] = v; // scalar constraint fields: format, title, pattern, ...
  }
  if (minimum !== undefined) out.minimum = minimum;
  if (maximum !== undefined) out.maximum = maximum;
  if (items !== undefined) out.items = convertSchemaNode(items);
  if (prefixItems !== undefined) out.prefixItems = (Array.isArray(prefixItems) ? prefixItems : [prefixItems]).map(convertSchemaNode);
  // Type: uppercase it; collapse an array of types ("null" -> nullable,
  // first concrete type wins); infer OBJECT when properties are present
  // (the proto allows properties only for OBJECT).
  let nullable = node.nullable === true;
  if (Array.isArray(node.type)) {
    const types = node.type.filter(t => typeof t === 'string');
    if (types.includes('null')) nullable = true;
    const concrete = types.filter(t => t !== 'null');
    if (concrete.length) out.type = concrete[0].toUpperCase();
  } else if (typeof node.type === 'string') {
    out.type = node.type.toUpperCase();
  }
  if (nullable) out.nullable = true;
  if (out.type === undefined && out.properties) out.type = 'OBJECT';
  // `required` may only name declared properties — the proto validates it
  // ("property is not defined"); with no properties there is nothing it
  // may name.
  if (Array.isArray(node.required)) {
    const declared = out.properties ? new Set(Object.keys(out.properties)) : new Set();
    const req = node.required.filter(n => typeof n === 'string' && declared.has(n));
    if (req.length) out.required = req;
  }
  return out;
};

const convertSchema = (schema) => {
  const converted = convertSchemaNode(resolveSchemaRefs(isSchemaRecord(schema) ? schema : {}));
  // The parameters root must declare OBJECT (tool-schema.ts's objectRoot:
  // providers reject a tool schema whose root does not declare it).
  if (converted.type === undefined) converted.type = 'OBJECT';
  return converted;
};

const textOf = (content) => {
  if (typeof content === 'string') return content;
  if (Array.isArray(content)) return content.filter(p => p && (p.type === 'text' || typeof p.text === 'string')).map(p => p.text || '').join('');
  return '';
};

const buildV1Request = (body) => {
  const { fullName, effort } = resolveModelAndEffort(body.model, body.reasoning_effort);
  const messages = Array.isArray(body.messages) ? body.messages : [];
  const systemParts = [];
  const contents = [];
  const toolNameById = new Map();
  for (const msg of messages) {
    if (!msg || !msg.role) continue;
    if (msg.role === 'system') {
      const t = textOf(msg.content);
      if (t) systemParts.push({ text: t });
    } else if (msg.role === 'user') {
      const parts = [];
      if (typeof msg.content === 'string') {
        if (msg.content) parts.push({ text: msg.content });
      } else if (Array.isArray(msg.content)) {
        for (const p of msg.content) {
          if (!p) continue;
          if (p.type === 'text' && p.text) parts.push({ text: p.text });
          else if (p.type === 'image_url' && p.image_url && p.image_url.url) {
            const url = p.image_url.url;
            const dm = url.match(/^data:([^;]+);base64,(.*)$/s);
            if (dm) parts.push({ inlineData: { mimeType: dm[1], data: dm[2] } });
            else parts.push({ fileData: { mimeType: p.image_url.mime_type || 'image/jpeg', fileUri: url } });
          }
        }
      }
      if (parts.length) contents.push({ role: 'user', parts });
    } else if (msg.role === 'assistant') {
      const parts = [];
      const t = textOf(msg.content);
      if (t) parts.push({ text: t });
      if (Array.isArray(msg.tool_calls)) {
        for (const tc of msg.tool_calls) {
          if (tc.id && tc.function && tc.function.name) toolNameById.set(tc.id, tc.function.name);
          let args = {};
          try {
            const parsedArgs = JSON.parse((tc.function && tc.function.arguments) || '{}');
            // functionCall.args is a proto Struct too: only a JSON object
            // may be sent (same code#2825 shape rule as functionResponse).
            if (parsedArgs && typeof parsedArgs === 'object' && !Array.isArray(parsedArgs)) args = parsedArgs;
          } catch {}
          const part = { functionCall: { name: tc.function && tc.function.name, args } };
          const sig = lookupSignature(tc.id);
          if (sig) part.thoughtSignature = sig;
          parts.push(part);
        }
      }
      if (parts.length) contents.push({ role: 'model', parts });
    } else if (msg.role === 'tool') {
      const name = msg.name || toolNameById.get(msg.tool_call_id) || 'tool';
      let result = textOf(msg.content) || (typeof msg.content === 'string' ? msg.content : '');
      // functionResponse.response is a proto Struct: it must be a JSON
      // object. A tool result that parses to an array or a scalar (a
      // list-shaped tool output is common) is wrapped, never sent raw —
      // a raw array 400s upstream ("Proto field is not repeating, cannot
      // start list", code#2825).
      let response;
      try {
        const parsed = JSON.parse(result);
        response = (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) ? parsed : { result: parsed };
      } catch { response = { result }; }
      contents.push({ role: 'user', parts: [{ functionResponse: { name, response } }] });
    }
  }
  const request = { contents };
  if (systemParts.length) request.systemInstruction = { role: 'user', parts: systemParts };
  if (Array.isArray(body.tools) && body.tools.length) {
    const decls = [];
    for (const t of body.tools) {
      const fn = t && t.type === 'function' ? t.function : t;
      if (fn && fn.name) decls.push({ name: fn.name, description: fn.description || '', parameters: convertSchema(fn.parameters || { type: 'object', properties: {} }) });
    }
    if (decls.length) request.tools = [{ functionDeclarations: decls }];
    if (body.tool_choice) {
      const mode = body.tool_choice === 'none' ? 'NONE' : body.tool_choice === 'auto' ? 'AUTO' : 'ANY';
      request.toolConfig = { functionCallingConfig: { mode } };
    }
  }
  const generationConfig = {
    maxOutputTokens: Number(body.max_tokens || body.max_completion_tokens || 65536),
    thinkingConfig: { includeThoughts: true, thinkingLevel: effort.toUpperCase() }
  };
  if (typeof body.temperature === 'number') generationConfig.temperature = body.temperature;
  if (typeof body.top_p === 'number') generationConfig.topP = body.top_p;
  request.generationConfig = generationConfig;
  // Negative int64 string, the shape the CLI sends (stable per conversation
  // there; this stateless door mints one per request).
  request.sessionId = '-' + (BigInt('0x' + crypto.randomBytes(8).toString('hex')) & ((1n << 62n) - 1n)).toString();
  return {
    fullName,
    payload: {
      project: PROJECT,
      requestId: 'agent/' + crypto.randomUUID() + '/' + Date.now() + '/' + crypto.randomUUID() + '/1',
      model: fullName,
      userAgent: 'antigravity',
      requestType: 'agent',
      request
    }
  };
};

// ---------------------------------------------------------------------------
// Upstream calls
// ---------------------------------------------------------------------------

const upstreamRequest = (urlPath, token, payload) => new Promise((resolve, reject) => {
  const base = new URL(UPSTREAM);
  const isHttps = base.protocol === 'https:';
  const lib = isHttps ? https : http;
  const bodyText = JSON.stringify(payload);
  const req = lib.request({
    protocol: base.protocol,
    hostname: base.hostname,
    port: base.port || (isHttps ? 443 : 80),
    path: (base.pathname === '/' ? '' : base.pathname) + urlPath,
    method: 'POST',
    headers: {
      'Authorization': 'Bearer ' + token,
      'User-Agent': binaryIntel().ua,
      'Content-Type': 'application/json',
      'Accept-Encoding': 'gzip',
      'Content-Length': Buffer.byteLength(bodyText)
    },
    timeout: TIMEOUT_MS
  }, (res) => {
    let stream = res;
    if (res.headers['content-encoding'] === 'gzip') stream = res.pipe(zlib.createGunzip());
    resolve({ status: res.statusCode, headers: res.headers, stream });
  });
  req.on('timeout', () => req.destroy(new Error('upstream timeout after ' + TIMEOUT_MS + 'ms')));
  req.on('error', reject);
  req.end(bodyText);
});

const upstreamJson = async (urlPath, token, payload) => {
  const { status, stream } = await upstreamRequest(urlPath, token, payload);
  const chunks = [];
  for await (const c of stream) chunks.push(c);
  const text = Buffer.concat(chunks).toString('utf8');
  if (status !== 200) throw new Error('upstream HTTP ' + status + ': ' + text.slice(0, 200));
  return JSON.parse(text);
};

// One generate call with a single auth retry (fresh keyring read + refresh).
const generate = async (payload) => {
  let token = await getAccessToken();
  let result = await upstreamRequest('/v1internal:streamGenerateContent?alt=sse', token, payload);
  if (result.status === 401) {
    log('upstream 401; re-reading keyring, refreshing, retrying once');
    result.stream.resume();
    token = await getAccessToken(true);
    result = await upstreamRequest('/v1internal:streamGenerateContent?alt=sse', token, payload);
  }
  return result;
};

// ---------------------------------------------------------------------------
// SSE consumption + OpenAI response shaping
// ---------------------------------------------------------------------------

const finishReasonMap = (fr, hasToolCalls) => {
  if (hasToolCalls) return 'tool_calls';
  switch (fr) {
    case 'STOP': return 'stop';
    case 'MAX_TOKENS': return 'length';
    case 'SAFETY': case 'RECITATION': case 'BLOCKLIST': case 'PROHIBITED_CONTENT': return 'content_filter';
    default: return 'stop';
  }
};

// Gemini 3 requires the thought_signature the model emitted alongside a
// function call to be echoed back in the history functionCall part, or the
// next turn fails HTTP 400 ("Function call is missing a thought_signature").
// OpenAI clients cannot carry it, so the door stashes it server-side keyed
// by the tool_call id it minted. In-memory only; a proxy restart between a
// call and its result loses it (the retried turn then 400s once, loudly).
const thoughtSignatures = new Map(); // tool_call id -> signature
const SIG_TTL_MS = 2 * 3600 * 1000;
const stashSignature = (id, sig) => {
  thoughtSignatures.set(id, { sig, at: Date.now() });
  if (thoughtSignatures.size > 1000) {
    const cutoff = Date.now() - SIG_TTL_MS;
    for (const [k, v] of thoughtSignatures) if (v.at < cutoff) thoughtSignatures.delete(k);
  }
};
const lookupSignature = (id) => {
  const e = id && thoughtSignatures.get(id);
  return e && Date.now() - e.at < SIG_TTL_MS ? e.sig : null;
};

// Consumes the upstream SSE stream, invoking onEvent({text, thought,
// toolCall, finishReason, usage}) for each meaningful delta. Tool call ids
// are minted here so thought signatures can be stashed against them.
// Returns the final { finishReason, usage }.
const consumeSse = async (stream, onEvent) => {
  let buffer = '';
  let usage = null;
  let finishReason = null;
  let lastSignature = null;
  const emittedCalls = [];
  const decoder = new TextDecoder();
  const handleData = (dataText) => {
    let obj;
    try { obj = JSON.parse(dataText); } catch { return; }
    if (obj.error) throw new Error('upstream stream error: ' + (obj.error.message || JSON.stringify(obj.error).slice(0, 200)));
    const resp = obj.response || obj;
    if (resp.usageMetadata) {
      usage = {
        prompt_tokens: resp.usageMetadata.promptTokenCount || 0,
        completion_tokens: (resp.usageMetadata.candidatesTokenCount || 0) + (resp.usageMetadata.thoughtsTokenCount || 0),
        total_tokens: resp.usageMetadata.totalTokenCount || 0
      };
    }
    const cand = resp.candidates && resp.candidates[0];
    if (!cand) return;
    if (cand.finishReason) finishReason = cand.finishReason;
    const parts = (cand.content && cand.content.parts) || [];
    for (const part of parts) {
      if (part.thoughtSignature) lastSignature = part.thoughtSignature;
      if (typeof part.text === 'string' && part.text) {
        onEvent({ [part.thought ? 'thought' : 'text']: part.text });
      }
      if (part.functionCall) {
        const call = { id: 'call_' + crypto.randomUUID().replace(/-/g, ''), name: part.functionCall.name, args: part.functionCall.args || {} };
        if (part.thoughtSignature) call.signature = part.thoughtSignature;
        emittedCalls.push(call);
        onEvent({ toolCall: call });
      }
    }
  };
  for await (const chunk of stream) {
    buffer += decoder.decode(chunk, { stream: true });
    let idx;
    while ((idx = buffer.indexOf('\n')) >= 0) {
      const line = buffer.slice(0, idx).trim();
      buffer = buffer.slice(idx + 1);
      if (line.startsWith('data:')) handleData(line.slice(5).trim());
    }
  }
  if (buffer.trim().startsWith('data:')) handleData(buffer.trim().slice(5).trim());
  // Attach the response's thought signature to its first function call
  // (the position the API checks), unless a call carried its own.
  if (emittedCalls.length) {
    const first = emittedCalls[0];
    const sig = first.signature || lastSignature;
    if (sig) stashSignature(first.id, sig);
  }
  return { finishReason, usage };
};

const readBody = (req) => new Promise((resolve, reject) => {
  const chunks = [];
  req.on('data', c => chunks.push(c));
  req.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
  req.on('error', reject);
});

const sendJson = (res, status, obj) => {
  const text = JSON.stringify(obj);
  res.writeHead(status, { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(text) });
  res.end(text);
};

const handleChatCompletions = async (req, res) => {
  const t0 = Date.now();
  let body;
  try { body = JSON.parse(await readBody(req) || '{}'); }
  catch { return sendJson(res, 400, { error: { message: 'invalid JSON body', type: 'invalid_request_error' } }); }
  const requestedModel = body.model || 'gemini-3.8-flash';
  const { fullName, payload } = buildV1Request(body);
  const id = 'chatcmpl-' + crypto.randomUUID();
  const created = Math.floor(Date.now() / 1000);
  let upstream;
  try {
    upstream = await generate(payload);
  } catch (e) {
    log('generate failed:', e.message);
    return sendJson(res, 502, { error: { message: e.message, type: 'upstream_error' } });
  }
  if (upstream.status !== 200) {
    const chunks = [];
    for await (const c of upstream.stream) chunks.push(c);
    const text = Buffer.concat(chunks).toString('utf8');
    log('upstream HTTP', upstream.status, 'for', fullName, ':', text.slice(0, 200));
    return sendJson(res, upstream.status, { error: { message: 'upstream HTTP ' + upstream.status + ': ' + text.slice(0, 300), type: 'upstream_error', code: upstream.status } });
  }
  const firstTokenMs = { value: null };
  if (body.stream) {
    res.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-cache', 'Connection': 'keep-alive' });
    const chunk = (delta, finish, usage) => {
      const c = { id, object: 'chat.completion.chunk', created, model: requestedModel,
        choices: [{ index: 0, delta, finish_reason: finish || null }] };
      if (usage) c.usage = usage;
      res.write('data: ' + JSON.stringify(c) + '\n\n');
    };
    chunk({ role: 'assistant', content: '' }, null, null);
    let toolIdx = 0;
    let sawTool = false;
    try {
      const { finishReason, usage } = await consumeSse(upstream.stream, (ev) => {
        if (firstTokenMs.value === null && (ev.text || ev.thought || ev.toolCall)) firstTokenMs.value = Date.now() - t0;
        if (ev.text) chunk({ content: ev.text }, null, null);
        if (ev.thought) chunk({ reasoning_content: ev.thought }, null, null);
        if (ev.toolCall) {
          sawTool = true;
          chunk({ tool_calls: [{ index: toolIdx++, id: ev.toolCall.id, type: 'function',
            function: { name: ev.toolCall.name, arguments: JSON.stringify(ev.toolCall.args) } }] }, null, null);
        }
      });
      chunk({}, finishReasonMap(finishReason, sawTool), usage);
    } catch (e) {
      log('stream error:', e.message);
      chunk({}, 'stop', null);
    }
    res.write('data: [DONE]\n\n');
    res.end();
    log(fullName, 'stream done in', Date.now() - t0, 'ms; first token', firstTokenMs.value, 'ms');
  } else {
    let text = '';
    let thought = '';
    const toolCalls = [];
    try {
      const { finishReason, usage } = await consumeSse(upstream.stream, (ev) => {
        if (firstTokenMs.value === null && (ev.text || ev.thought || ev.toolCall)) firstTokenMs.value = Date.now() - t0;
        if (ev.text) text += ev.text;
        if (ev.thought) thought += ev.thought;
        if (ev.toolCall) toolCalls.push({ id: ev.toolCall.id, type: 'function',
          function: { name: ev.toolCall.name, arguments: JSON.stringify(ev.toolCall.args) } });
      });
      const message = { role: 'assistant', content: text || null };
      if (thought) message.reasoning_content = thought;
      if (toolCalls.length) message.tool_calls = toolCalls;
      const out = { id, object: 'chat.completion', created, model: requestedModel,
        choices: [{ index: 0, message, finish_reason: finishReasonMap(finishReason, toolCalls.length > 0) }] };
      if (usage) out.usage = usage;
      log(fullName, 'done in', Date.now() - t0, 'ms; first token', firstTokenMs.value, 'ms');
      return sendJson(res, 200, out);
    } catch (e) {
      log('consume failed:', e.message);
      return sendJson(res, 502, { error: { message: e.message, type: 'upstream_error' } });
    }
  }
};

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

const stateDir = () => path.join(process.env.XDG_STATE_HOME || path.join(os.homedir(), '.local', 'state'), 'antigravity-direct');

const server = http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url, 'http://' + HOST);
    if (req.method === 'GET' && (url.pathname === '/v1/models' || url.pathname === '/models')) {
      const data = [];
      for (const m of modelCatalog) {
        data.push({ id: m.id, object: 'model', created: 0, owned_by: 'antigravity', name: m.name });
        for (const e of m.efforts) data.push({ id: m.id + '-' + e, object: 'model', created: 0, owned_by: 'antigravity', name: m.name + ' (' + e + ')' });
      }
      return sendJson(res, 200, { object: 'list', data });
    }
    if (req.method === 'POST' && (url.pathname === '/v1/chat/completions' || url.pathname === '/chat/completions')) {
      return await handleChatCompletions(req, res);
    }
    if (req.method === 'GET' && url.pathname === '/healthz') {
      return sendJson(res, 200, { ok: true, upstream: UPSTREAM, models: modelCatalog.map(m => m.id) });
    }
    if (req.method === 'GET' && url.pathname === '/') {
      return sendJson(res, 200, { service: 'antigravity-direct', upstream: UPSTREAM, port: PORT });
    }
    return sendJson(res, 404, { error: { message: 'not found: ' + req.method + ' ' + url.pathname, type: 'invalid_request_error' } });
  } catch (e) {
    log('handler error:', e && e.stack || e);
    if (!res.headersSent) sendJson(res, 500, { error: { message: String(e && e.message || e), type: 'internal_error' } });
    else res.end();
  }
});

server.listen(PORT, HOST, () => {
  log('antigravity-direct listening on http://' + HOST + ':' + PORT, '-> upstream', UPSTREAM);
  try {
    fs.mkdirSync(stateDir(), { recursive: true });
    fs.writeFileSync(path.join(stateDir(), 'endpoint.json'),
      JSON.stringify({ host: HOST, port: PORT, pid: process.pid, url: 'http://' + HOST + ':' + PORT }) + '\n');
  } catch {}
  refreshCatalog();
});
