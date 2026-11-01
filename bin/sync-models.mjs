#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { execSync } from 'node:child_process';

const DRY_RUN = process.argv.includes('--dry-run');
const VERBOSE = process.argv.includes('--verbose') || process.argv.includes('-v');

const ROOT_DIR = path.resolve(import.meta.dirname, '..');
const HOME_DIR = process.env.HOME || '/home/user';
// Config resolution (#591): in a repo checkout, sync the repo's own
// .opencode/opencode.jsonc. When home-manager installs this script it lands
// alone in the Nix store, so ROOT_DIR resolves to /nix and the repo-relative
// path never exists — fall back to the deployed user config, which is the
// file the sync-models service actually maintains. $OPENCODE_CONFIG_PATH
// overrides both.
const REPO_CONFIG_PATH = path.join(ROOT_DIR, '.opencode/opencode.jsonc');
const USER_CONFIG_PATH = path.join(HOME_DIR, '.config/opencode/opencode.jsonc');
const OPENCODE_CONFIG_PATH =
  process.env.OPENCODE_CONFIG_PATH ||
  (fs.existsSync(REPO_CONFIG_PATH) ? REPO_CONFIG_PATH : USER_CONFIG_PATH);
const POLICY_PATH = path.join(import.meta.dirname, 'sync-models.policy.json');
const CODEX_CACHE_PATH = path.join(HOME_DIR, '.codex/models_cache.json');
const GEMINI_CACHE_PATH = path.join(HOME_DIR, '.gemini/antigravity_models_cache.json');
const LS_LOG_PATH = path.join(HOME_DIR, '.config/Antigravity/logs/language_server.log');

console.log(`[sync-models] Mode: ${DRY_RUN ? 'DRY-RUN (no files will be changed)' : 'APPLY'}`);

// -----------------------------------------------------------------------------
// 1. Fetch models.dev catalog
// -----------------------------------------------------------------------------
async function fetchModelsDev() {
  console.log('[sync-models] Fetching upstream models from https://models.dev/api.json...');
  try {
    const res = await fetch('https://models.dev/api.json', { signal: AbortSignal.timeout(10000) });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = await res.json();
    console.log(`[sync-models] Successfully loaded models.dev: ${Object.keys(data).length} providers`);
    return data;
  } catch (err) {
    console.warn(`[sync-models] Warning: Failed to fetch models.dev (${err.message}). Continuing with local sources.`);
    return null;
  }
}

// The language server listens on a random port each time it starts. Its HTTP
// port is logged rather than passed as a flag, and an HTTPS listener is logged
// the same way, so try each candidate port with each candidate token and keep
// the first one that returns a catalog.
async function fetchLiveCatalog() {
  const tokens = [];
  const ports = [];

  if (process.env.ANTIGRAVITY_CSRF_TOKEN) tokens.push(process.env.ANTIGRAVITY_CSRF_TOKEN);

  try {
    const ps = execSync('ps aux | grep language_server | grep -v grep', { encoding: 'utf-8' });
    for (const line of ps.split('\n')) {
      const token = line.match(/--csrf_token\s+([a-zA-Z0-9_-]+)/);
      if (token) tokens.push(token[1]);
      const port = line.match(/--http_server_port\s+(\d+)/);
      if (port && Number(port[1]) > 0) ports.push(Number(port[1]));
    }
  } catch {}

  try {
    const log = fs.readFileSync(LS_LOG_PATH, 'utf-8').split('\n').reverse();
    for (const line of log) {
      const m = line.match(/listening on random port at (\d+) for HTTP\b/);
      if (m) ports.push(Number(m[1]));
    }
  } catch {}

  for (const port of new Set(ports)) {
    for (const token of new Set(tokens)) {
      try {
        const res = await fetch(`http://127.0.0.1:${port}/exa.language_server_pb.LanguageServerService/GetAvailableModels`, {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
            'x-codeium-csrf-token': token
          },
          body: '{}',
          signal: AbortSignal.timeout(3000)
        });
        if (!res.ok) continue;
        const models = (await res.json()).response?.models || {};
        if (Object.keys(models).length > 0) return models;
      } catch {}
    }
  }
  return null;
}

// -----------------------------------------------------------------------------
// 2. Discover Antigravity Models (via Language Server RPC or agy CLI)
// -----------------------------------------------------------------------------
async function discoverAntigravityModels() {
  console.log('[sync-models] Discovering Antigravity models...');
  const discovered = [];

  // Try 1: Language Server RPC GetAvailableModels
  const models = await fetchLiveCatalog();
  if (models) {
    for (const [key, val] of Object.entries(models)) {
      if (key.startsWith('tab_') || key.startsWith('chat_')) continue;
      discovered.push({
        id: key,
        name: val.displayName || key,
        modelEnum: val.model,
        maxTokens: val.maxTokens || 65536,
        supportsThinking: Boolean(val.supportsThinking),
        source: 'language-server'
      });
    }
    if (discovered.length > 0) {
      console.log(`[sync-models] Discovered ${discovered.length} models from Antigravity Language Server`);
      return discovered;
    }
  }

  // Try 2: agy CLI models command
  try {
    const agyOut = execSync('HOME=/home/user/.gemini/agy-proxy-env agy models 2>&1', { encoding: 'utf-8' });
    const lines = agyOut.split('\n');
    for (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed || trimmed.includes('Fetching available models')) continue;
      const parts = trimmed.split(/\s{2,}/);
      if (parts.length >= 2) {
        const id = parts[0].trim();
        const name = parts[1].trim();
        discovered.push({ id, name, source: 'agy-cli' });
      } else if (parts.length === 1 && !parts[0].includes(' ')) {
        discovered.push({ id: parts[0].trim(), name: parts[0].trim(), source: 'agy-cli' });
      }
    }
    if (discovered.length > 0) {
      console.log(`[sync-models] Discovered ${discovered.length} models from agy CLI`);
      return discovered;
    }
  } catch (err) {
    console.warn(`[sync-models] agy models command failed: ${err.message}`);
  }

  return discovered;
}

// -----------------------------------------------------------------------------
// 3. Discover Codex Models (via ~/.codex/models_cache.json)
// -----------------------------------------------------------------------------
function discoverCodexModels() {
  console.log('[sync-models] Discovering Codex models...');
  if (!fs.existsSync(CODEX_CACHE_PATH)) {
    console.warn(`[sync-models] Codex cache file not found at ${CODEX_CACHE_PATH}`);
    return [];
  }
  try {
    const raw = fs.readFileSync(CODEX_CACHE_PATH, 'utf-8');
    const data = JSON.parse(raw);
    const models = (data.models || []).map(m => ({
      slug: m.slug,
      name: m.display_name || m.name || m.slug,
      description: m.description || '',
      defaultEffort: m.default_reasoning_effort || 'medium'
    }));
    console.log(`[sync-models] Discovered ${models.length} models from Codex cache`);
    return models;
  } catch (err) {
    console.warn(`[sync-models] Failed to parse Codex cache: ${err.message}`);
    return [];
  }
}

// -----------------------------------------------------------------------------
// Model policy (bin/sync-models.policy.json). Per provider, keep only the newest
// model in each listed family and drop everything else. Exclusions are applied
// before picking the newest, so excluding the newest promotes the one before it.
// -----------------------------------------------------------------------------
function loadPolicy() {
  if (!fs.existsSync(POLICY_PATH)) return { families: {}, providers: {} };
  const policy = JSON.parse(fs.readFileSync(POLICY_PATH, 'utf-8'));
  for (const [name, provider] of Object.entries(policy.providers || {})) {
    for (const family of provider.keep || []) {
      if (!Object.hasOwn(policy.families || {}, family) || !policy.families[family]) {
        throw new Error(`sync-models.policy.json: ${name} keeps unknown family "${family}"`);
      }
    }
  }
  return policy;
}

const compareVersions = (a, b) => {
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const d = (a[i] ?? 0) - (b[i] ?? 0);
    if (d !== 0) return d;
  }
  return 0;
};

// The model ids one provider should carry, given its policy.
function allowedModelIds(policy, providerPolicy, ids) {
  const excluded = new Set(providerPolicy.exclude || []);
  const newest = new Map();
  for (const id of ids) {
    if (excluded.has(id)) continue;
    for (const family of providerPolicy.keep || []) {
      const match = id.match(new RegExp(policy.families[family]));
      if (!match) continue;
      if (match[1] === undefined) continue;
      const version = match[1].replace(/-/g, '.').split('.').map(Number);
      const current = newest.get(family);
      if (!current || compareVersions(version, current.version) > 0) {
        newest.set(family, { id, version });
      }
      break;
    }
  }
  return new Set([...newest.values()].map(n => n.id));
}

// -----------------------------------------------------------------------------
// Helper: Clean JSONC parser (for reading only; it drops comments)
// -----------------------------------------------------------------------------
function parseJsonc(text) {
  // #704: one string-aware scan. Comments and trailing commas are only
  // special OUTSIDE string literals; string contents are copied verbatim.
  // The old line-based stripper threw on multi-line block comments and on
  // ' // ' inside strings, and silently ate ', ]' sequences inside strings
  // (the trailing-comma regex could not tell a string from structure).
  let stripped = '';
  let i = 0;
  while (i < text.length) {
    const ch = text[i];
    if (ch === '"') {
      let j = i + 1;
      while (j < text.length && text[j] !== '"') j += text[j] === '\\' ? 2 : 1;
      stripped += text.slice(i, j + 1);
      i = j + 1;
    } else if (ch === '/' && text[i + 1] === '/') {
      while (i < text.length && text[i] !== '\n') i++;
    } else if (ch === '/' && text[i + 1] === '*') {
      const end = text.indexOf('*/', i + 2);
      i = end < 0 ? text.length : end + 2;
      stripped += ' '; // keep neighbouring tokens separated
    } else {
      stripped += ch;
      i++;
    }
  }
  // Drop trailing commas, still string-aware (a ', ]' inside a string is
  // content, not a trailing comma).
  let cleaned = '';
  let inString = false;
  for (let k = 0; k < stripped.length; k++) {
    const ch = stripped[k];
    if (inString) {
      cleaned += ch;
      if (ch === '\\') cleaned += stripped[++k] ?? '';
      else if (ch === '"') inString = false;
    } else if (ch === '"') {
      inString = true;
      cleaned += ch;
    } else if (ch === ',') {
      let m = k + 1;
      while (m < stripped.length && /\s/.test(stripped[m])) m++;
      if (stripped[m] !== '}' && stripped[m] !== ']') cleaned += ch;
    } else {
      cleaned += ch;
    }
  }
  return JSON.parse(cleaned);
}

// -----------------------------------------------------------------------------
// Helper: comment-preserving JSONC edits. The file is edited as text, one member
// at a time, so every byte outside the edited span (comments included) is kept.
// parseJsoncTree records where each value sits in the source.
// -----------------------------------------------------------------------------
function parseJsoncTree(text) {
  let i = 0;
  const fail = () => {
    throw new Error(`malformed JSONC at offset ${i}`);
  };
  const skip = () => {
    for (;;) {
      while (i < text.length && /\s/.test(text[i])) i++;
      if (text.startsWith('//', i)) {
        while (i < text.length && text[i] !== '\n') i++;
      } else if (text.startsWith('/*', i)) {
        const end = text.indexOf('*/', i + 2);
        i = end < 0 ? text.length : end + 2;
      } else {
        return;
      }
    }
  };
  const readString = () => {
    if (text[i] !== '"') fail();
    const start = i;
    i++;
    while (i < text.length && text[i] !== '"') i += text[i] === '\\' ? 2 : 1;
    if (i >= text.length) fail();
    i++;
    return { start, end: i, value: JSON.parse(text.slice(start, i)) };
  };
  const readValue = () => {
    skip();
    if (i >= text.length) fail();
    const start = i;
    if (text[i] === '{') {
      i++;
      const members = [];
      skip();
      while (text[i] !== '}') {
        if (i >= text.length) fail();
        const keyStart = i;
        const key = readString().value;
        skip();
        if (text[i] !== ':') fail();
        i++;
        const value = readValue();
        members.push({ key, keyStart, value });
        skip();
        if (text[i] === ',') {
          i++;
          skip();
        }
      }
      i++;
      return { type: 'object', start, end: i, members };
    }
    if (text[i] === '[') {
      i++;
      skip();
      while (text[i] !== ']') {
        if (i >= text.length) fail();
        readValue();
        skip();
        if (text[i] === ',') {
          i++;
          skip();
        }
      }
      i++;
      return { type: 'array', start, end: i };
    }
    if (text[i] === '"') return { type: 'scalar', start, end: readString().end };
    while (i < text.length && !/[\s,\]}\/]/.test(text[i])) i++;
    return { type: 'scalar', start, end: i };
  };
  const root = readValue();
  if (root.type !== 'object') fail();
  return root;
}

// The line's leading whitespace, used so inserted text matches its neighbours.
const lineIndent = (text, pos) => {
  const lineStart = text.lastIndexOf('\n', pos - 1) + 1;
  return text.slice(lineStart).match(/^[ \t]*/)[0];
};

const jsoncText = (value, indent) => JSON.stringify(value, null, 2).split('\n').join(`\n${indent}`);

function setJsoncMember(text, object, key, value) {
  const member = object.members.find(m => m.key === key);
  if (member) {
    const indent = lineIndent(text, member.value.start);
    return text.slice(0, member.value.start) + jsoncText(value, indent) + text.slice(member.value.end);
  }
  const objectIndent = lineIndent(text, object.start);
  const memberIndent = `${objectIndent}  `;
  const entry = `${JSON.stringify(key)}: ${jsoncText(value, memberIndent)}`;
  if (object.members.length === 0) {
    return `${text.slice(0, object.start + 1)}\n${memberIndent}${entry}\n${objectIndent}${text.slice(object.start + 1)}`;
  }
  // After the last value, so any comma or comment that followed it stays put.
  const at = object.members.at(-1).value.end;
  return `${text.slice(0, at)},\n${memberIndent}${entry}${text.slice(at)}`;
}

// Sets the value at `keys`, creating missing parents, leaving comments alone.
function setJsoncPath(text, keys, value) {
  let object = parseJsoncTree(text);
  for (let depth = 0; depth < keys.length - 1; depth++) {
    const member = object.members.find(m => m.key === keys[depth]);
    if (!member) {
      let nested = value;
      for (let d = keys.length - 1; d > depth + 1; d--) nested = { [keys[d]]: nested };
      return setJsoncMember(text, object, keys[depth], { [keys[depth + 1]]: nested });
    }
    object = member.value;
  }
  return setJsoncMember(text, object, keys.at(-1), value);
}

// Removes the member at `keys` by deleting its lines (and its comma). Comments
// on other lines are kept.
function removeJsoncPath(text, keys) {
  let object = parseJsoncTree(text);
  for (const key of keys.slice(0, -1)) {
    object = object.members.find(m => m.key === key)?.value;
    if (!object) return text;
  }
  const member = object.members.find(m => m.key === keys.at(-1));
  if (!member) return text;
  const lineStart = text.lastIndexOf('\n', member.keyStart - 1) + 1;
  if (text.slice(lineStart, member.keyStart).trim() !== '') {
    throw new Error(`cannot remove ${keys.join('/')}: it shares a line with other JSON`);
  }
  let end = member.value.end;
  while (text[end] === ' ' || text[end] === '\t') end++;
  if (text[end] === ',') end++;
  const newline = text.indexOf('\n', end);
  return text.slice(0, lineStart) + text.slice(newline < 0 ? text.length : newline + 1);
}

// -----------------------------------------------------------------------------
// Main Sync Runner
// -----------------------------------------------------------------------------
async function run() {
  const modelsDev = await fetchModelsDev();
  const agyModels = await discoverAntigravityModels();
  const codexModels = discoverCodexModels();
  const policy = loadPolicy();

  if (!fs.existsSync(OPENCODE_CONFIG_PATH)) {
    console.error(`[sync-models] Error: ${OPENCODE_CONFIG_PATH} does not exist!`);
    process.exit(1);
  }

  const origConfigText = fs.readFileSync(OPENCODE_CONFIG_PATH, 'utf-8');
  let config;
  try {
    config = parseJsonc(origConfigText);
  } catch (err) {
    console.error(`[sync-models] Error parsing ${OPENCODE_CONFIG_PATH}: ${err.message}`);
    process.exit(1);
  }

  let totalAdded = 0;
  let totalUpdated = 0;
  let totalRemoved = 0;
  // Every change is recorded here and applied to the text at the end, so the
  // file keeps its comments and formatting.
  const edits = [];

  // 1. Sync antigravity-direct (the CLI-wire door; antigravity-agy &
  // antigravity-ls retired 2026-10-30 — see home/common/default.nix)
  if (agyModels.length > 0) {
    // Write local cache for proxies
    if (!DRY_RUN) {
      try {
        fs.mkdirSync(path.dirname(GEMINI_CACHE_PATH), { recursive: true });
        fs.writeFileSync(GEMINI_CACHE_PATH, JSON.stringify({
          updatedAt: new Date().toISOString(),
          models: agyModels
        }, null, 2));
        console.log(`[sync-models] Saved Antigravity models cache to ${GEMINI_CACHE_PATH}`);
      } catch {}
    }

    // Group models by base name (matching longest suffix first)
    const effortSuffixes = ['-extra-low', '-medium', '-high', '-low'];
    const grouped = {};

    for (const m of agyModels) {
      let baseId = m.id;
      let effort = null;
      for (const eff of effortSuffixes) {
        if (baseId.endsWith(eff)) {
          effort = eff.slice(1);
          baseId = baseId.slice(0, -eff.length);
          break;
        }
      }
      if (!grouped[baseId]) {
        grouped[baseId] = {
          baseId,
          name: m.name.replace(/\s*\((Low|Medium|High|Extra-Low)\)$/i, ''),
          efforts: new Set(),
          variants: {}
        };
      }
      if (effort) {
        grouped[baseId].efforts.add(effort);
      }
    }

    const defaultLimits = (id) => {
      if (id.includes('claude')) return { context: 1000000, output: 64000 };
      if (id.includes('gpt-oss')) return { context: 131072, output: 16384 };
      return { context: 1048576, output: 65536 };
    };

    for (const targetProvider of ['antigravity-direct']) {
      if (!config.provider[targetProvider]) continue;
      const prov = config.provider[targetProvider];
      prov.models = prov.models || {};
      const modelsPath = ['provider', targetProvider, 'models'];

      // A provider with a policy carries only what the policy allows.
      const providerPolicy = policy.providers?.[targetProvider];
      const allowed = providerPolicy
        ? allowedModelIds(policy, providerPolicy, [...new Set([...Object.keys(grouped), ...Object.keys(prov.models)])])
        : null;
      if (allowed) {
        for (const id of Object.keys(prov.models)) {
          if (allowed.has(id)) continue;
          console.log(`[sync-models] - Removing ${targetProvider}/${id} (not in its policy)`);
          delete prov.models[id];
          edits.push({ remove: [...modelsPath, id] });
          totalRemoved++;
        }
      }

      for (const [baseId, info] of Object.entries(grouped)) {
        if (allowed && !allowed.has(baseId)) continue;
        const limits = defaultLimits(baseId);
        const variants = {};
        for (const eff of Array.from(info.efforts).sort()) {
          variants[eff] = { options: { reasoning_effort: eff } };
        }

        const modelEntry = {
          name: `${info.name} (${targetProvider === 'antigravity-ls' ? 'Direct LS' : targetProvider === 'antigravity-agy' ? 'agy' : 'direct'})`,
          limit: limits
        };
        if (Object.keys(variants).length > 0) {
          modelEntry.variants = variants;
        }

        if (!prov.models[baseId]) {
          console.log(`[sync-models] + Adding ${targetProvider}/${baseId}`);
          prov.models[baseId] = modelEntry;
          edits.push({ set: [...modelsPath, baseId], value: modelEntry });
          totalAdded++;
        } else {
          let changed = false;
          if (Object.keys(variants).length > 0 && !prov.models[baseId].variants) {
            prov.models[baseId].variants = variants;
            edits.push({ set: [...modelsPath, baseId, 'variants'], value: variants });
            changed = true;
          }
          if (prov.models[baseId].limit?.context !== limits.context) {
            prov.models[baseId].limit = limits;
            edits.push({ set: [...modelsPath, baseId, 'limit'], value: limits });
            changed = true;
          }
          if (changed) {
            if (VERBOSE) console.log(`[sync-models] ~ Updated ${targetProvider}/${baseId}`);
            totalUpdated++;
          }
        }
      }
    }
  }

  // 2. Sync openai-codex
  if (codexModels.length > 0 && config.provider['openai-codex']) {
    const codexProv = config.provider['openai-codex'];
    codexProv.models = codexProv.models || {};
    const modelsPath = ['provider', 'openai-codex', 'models'];

    const codexEfforts = ['low', 'medium', 'high', 'xhigh', 'max', 'ultra'];
    const codexVariants = {};
    for (const eff of codexEfforts) {
      codexVariants[eff] = { options: { reasoning_effort: eff } };
    }

    for (const m of codexModels) {
      if (m.slug === 'codex-auto-review') continue;
      const entry = {
        name: m.name.includes('(') ? m.name : `${m.name} (Codex CLI)`,
        variants: codexVariants,
        limit: { context: 1000000, output: 128000 }
      };

      if (!codexProv.models[m.slug]) {
        console.log(`[sync-models] + Adding openai-codex/${m.slug}`);
        codexProv.models[m.slug] = entry;
        edits.push({ set: [...modelsPath, m.slug], value: entry });
        totalAdded++;
      } else {
        if (codexProv.models[m.slug].limit?.context !== 1000000) {
          codexProv.models[m.slug].limit = { context: 1000000, output: 128000 };
          edits.push({ set: [...modelsPath, m.slug, 'limit'], value: { context: 1000000, output: 128000 } });
          totalUpdated++;
        }
      }
    }
  }

  // 3. Sync models.dev for cloud providers (google, openai, cloudflare-workers-ai)
  if (modelsDev) {
    // Sync Google
    if (config.provider.google && modelsDev.google?.models) {
      const gProv = config.provider.google;
      gProv.models = gProv.models || {};
      for (const [mId, mSpec] of Object.entries(modelsDev.google.models)) {
        if (!gProv.models[mId]) {
          const entry = {
            name: mSpec.name,
            limit: mSpec.limit || { context: 1048576, output: 65536 }
          };
          if (mSpec.reasoning) {
            entry.variants = {
              low: { options: { thinkingLevel: 'low' } },
              high: { options: { thinkingLevel: 'high' } }
            };
          }
          console.log(`[sync-models] + Adding google/${mId}`);
          gProv.models[mId] = entry;
          edits.push({ set: ['provider', 'google', 'models', mId], value: entry });
          totalAdded++;
        }
      }
    }

    // Sync OpenAI
    if (config.provider.openai && modelsDev.openai?.models) {
      const oProv = config.provider.openai;
      oProv.models = oProv.models || {};
      for (const [mId, mSpec] of Object.entries(modelsDev.openai.models)) {
        if (!oProv.models[mId]) {
          const entry = {
            name: mSpec.name,
            limit: mSpec.limit || { context: 128000, output: 16384 }
          };
          if (mSpec.reasoning) {
            entry.variants = {
              low: { options: { reasoning_effort: 'low' } },
              medium: { options: { reasoning_effort: 'medium' } },
              high: { options: { reasoning_effort: 'high' } }
            };
          }
          if (VERBOSE) console.log(`[sync-models] + Adding openai/${mId}`);
          oProv.models[mId] = entry;
          edits.push({ set: ['provider', 'openai', 'models', mId], value: entry });
          totalAdded++;
        }
      }
    }

    // Sync Cloudflare Workers AI
    if (config.provider['cloudflare-gateway'] && modelsDev['cloudflare-workers-ai']?.models) {
      const cfProv = config.provider['cloudflare-gateway'];
      cfProv.models = cfProv.models || {};
      for (const [mId, mSpec] of Object.entries(modelsDev['cloudflare-workers-ai'].models)) {
        if (!cfProv.models[mId]) {
          const entry = {
            name: mSpec.name,
            limit: mSpec.limit || { context: 131072, output: 8192 }
          };
          if (mSpec.reasoning) {
            entry.variants = {
              low: { options: { reasoning_effort: 'low' } },
              medium: { options: { reasoning_effort: 'medium' } },
              high: { options: { reasoning_effort: 'high' } }
            };
          }
          if (VERBOSE) console.log(`[sync-models] + Adding cloudflare-gateway/${mId}`);
          cfProv.models[mId] = entry;
          edits.push({ set: ['provider', 'cloudflare-gateway', 'models', mId], value: entry });
          totalAdded++;
        }
      }
    }
  }

  console.log(`[sync-models] Summary: ${totalAdded} models added, ${totalUpdated} models updated, ${totalRemoved} models removed.`);

  if (totalAdded === 0 && totalUpdated === 0 && totalRemoved === 0) {
    console.log('[sync-models] All providers are already up-to-date!');
    return;
  }

  let newText = origConfigText;
  for (const edit of edits) {
    newText = edit.remove
      ? removeJsoncPath(newText, edit.remove)
      : setJsoncPath(newText, edit.set, edit.value);
  }
  // Refuse to write anything that no longer parses.
  parseJsoncTree(newText);
  parseJsonc(newText);

  if (DRY_RUN) {
    console.log('[sync-models] Dry run complete. No files written.');
    return;
  }

  const backupPath = `${OPENCODE_CONFIG_PATH}.backup`;
  fs.writeFileSync(backupPath, origConfigText);
  console.log(`[sync-models] Created backup at ${backupPath}`);

  fs.writeFileSync(OPENCODE_CONFIG_PATH, newText);
  console.log(`[sync-models] Successfully updated ${OPENCODE_CONFIG_PATH}!`);

  try {
    execSync('opencode reload', { stdio: 'ignore', timeout: 5000 });
    console.log('[sync-models] Triggered opencode configuration reload.');
  } catch {
    // Ignored if OpenCode background service is not running
  }
}

run().catch(err => {
  console.error('[sync-models] Fatal error:', err);
  process.exit(1);
});
