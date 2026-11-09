#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { execSync } from 'node:child_process';

// #2953 (define, don't assign): store an externally-spelled key with
// define semantics, so "__proto__" becomes an own property instead of
// invoking the Object.prototype setter and "constructor"/"toString"
// shadow like any other key.
const defineValue = (obj, key, value) => {
  Object.defineProperty(obj, key, { value, writable: true, enumerable: true, configurable: true });
  return value;
};


const KNOWN_ARGS = new Set(['--dry-run', '--verbose', '-v']);
const unknownArgs = process.argv.slice(2).filter(arg => !KNOWN_ARGS.has(arg));
if (unknownArgs.length > 0) {
  console.error(`[sync-models] Error: unknown argument '${unknownArgs[0]}'`);
  console.error('Usage: sync-models.mjs [--dry-run] [--verbose|-v]');
  process.exit(2);
}

const DRY_RUN = process.argv.includes('--dry-run');
const VERBOSE = process.argv.includes('--verbose') || process.argv.includes('-v');

const ROOT_DIR = path.resolve(import.meta.dirname, '..');
const HOME_DIR = process.env.HOME || '/home/user';
// Config resolution (#591, #4544): the model catalogs this script maintains
// live in the runtime overlay (~/.local/state/opencode/opencode-runtime.jsonc),
// which the OpenCode server layers over the tracked configured default via
// OPENCODE_CONFIG. The tracked file carries deliberate configuration only,
// so these automatic rewrites never dirty the repo tree. $OPENCODE_CONFIG_PATH
// overrides everything. When no runtime overlay exists yet (pre-split
// installs), fall back to the historical chain: in a repo checkout, sync the
// repo's own .opencode/opencode.jsonc; when home-manager installs this script
// it lands alone in the Nix store, so ROOT_DIR resolves to /nix and the
// repo-relative path never exists — fall back to the deployed user config,
// which is the file the sync-models service actually maintains.
const RUNTIME_CONFIG_PATH = path.join(
  HOME_DIR,
  '.local/state/opencode/opencode-runtime.jsonc',
);
const REPO_CONFIG_PATH = path.join(ROOT_DIR, '.opencode/opencode.jsonc');
const USER_CONFIG_PATH = path.join(HOME_DIR, '.config/opencode/opencode.jsonc');
const OPENCODE_CONFIG_PATH =
  process.env.OPENCODE_CONFIG_PATH ||
  (fs.existsSync(RUNTIME_CONFIG_PATH)
    ? RUNTIME_CONFIG_PATH
    : fs.existsSync(REPO_CONFIG_PATH)
      ? REPO_CONFIG_PATH
      : USER_CONFIG_PATH);
const POLICY_PATH = path.join(import.meta.dirname, 'sync-models.policy.json');
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
      // #6893: a capture with a non-numeric component (e.g. "2-beta")
      // maps that component to NaN, and every comparison against NaN
      // is false — the "newest" pick would silently be whichever id
      // was enumerated first. Such an id is deterministically
      // excluded from selection: it can never be a family's newest
      // model, in any input order.
      if (version.some(component => Number.isNaN(component))) break;
      const current = newest.get(family);
      // Equal versions tie-break on the id itself, so the pick is
      // the same set for every permutation of the same id list.
      if (!current || compareVersions(version, current.version) > 0 ||
          (compareVersions(version, current.version) === 0 && id > current.id)) {
        newest.set(family, { id, version });
      }
      break;
    }
  }
  return new Set([...newest.values()].map(n => n.id));
}

// -----------------------------------------------------------------------------
// Section resolution for sync-owned providers (#7333). A provider rename
// leaves its catalog under the pre-rename id: the 2026-11-07 rename
// (opencode#1823) moved antigravity-direct -> _antigravity, and the runtime
// overlay may still carry only the old key. Looking up the target alone then
// silently no-ops and the run reports "up to date" while the catalog is
// orphaned. Resolution migrates the legacy section, or reports the absence,
// so the caller never mistakes "not there" for "in sync".
// -----------------------------------------------------------------------------
const PROVIDER_ALIASES = {
  _antigravity: ['antigravity-direct', 'antigravity-agy', 'antigravity-ls'],
  _claude: ['claude-code'],
  _vibe: ['vibe-direct'],
  _commandcode: ['commandcode'],
  _muse: ['muse-direct'],
};

// Returns { status, provider, migratedFrom } with status 'present',
// 'migrated' or 'absent'. Moving the key is an in-memory change; the caller
// records the matching text edit (renameJsoncProvider) when it applies.
function resolveProviderSection(config, targetProvider) {
  config.provider = config.provider || {};
  if (Object.hasOwn(config.provider, targetProvider)) {
    return { status: 'present', provider: config.provider[targetProvider], migratedFrom: null };
  }
  for (const legacyId of PROVIDER_ALIASES[targetProvider] || []) {
    if (!Object.hasOwn(config.provider, legacyId)) continue;
    const legacy = config.provider[legacyId];
    if (legacy === null || typeof legacy !== 'object') continue;
    defineValue(config.provider, targetProvider, legacy);
    delete config.provider[legacyId];
    return { status: 'migrated', provider: legacy, migratedFrom: legacyId };
  }
  return { status: 'absent', provider: null, migratedFrom: null };
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
    if (member.value.type !== 'object') {
      throw new Error(
        `cannot set ${keys.join('/')}: ${keys.slice(0, depth + 1).join('/')} is a ${member.value.type}, expected object`,
      );
    }
    object = member.value;
  }
  return setJsoncMember(text, object, keys.at(-1), value);
}

// Removes the member at `keys` by deleting its lines (and its comma). Comments
// on other lines are kept; an inline comment on the removed member is refused
// rather than silently discarded.
function removeJsoncPath(text, keys) {
  let object = parseJsoncTree(text);
  for (const [i, key] of keys.slice(0, -1).entries()) {
    const next = object.members.find(m => m.key === key)?.value;
    if (!next) return text;
    if (next.type !== 'object') {
      throw new Error(
        `cannot remove ${keys.join('/')}: ${keys.slice(0, i + 1).join('/')} is a ${next.type}, expected object`,
      );
    }
    object = next;
  }
  const member = object.members.find(m => m.key === keys.at(-1));
  if (!member) return text;
  const lineStart = text.lastIndexOf('\n', member.keyStart - 1) + 1;
  if (text.slice(lineStart, member.keyStart).trim() !== '') {
    throw new Error(`cannot remove ${keys.join('/')}: it shares a line with other JSON`);
  }
  const lineEnd = text.indexOf('\n', member.value.end);
  const memberLineEnd = lineEnd < 0 ? text.length : lineEnd;
  const trailing = text.slice(member.value.end, memberLineEnd);
  if (trailing.includes('//') || trailing.includes('/*')) {
    const error = new Error(
      `cannot remove ${keys.join('/')}: inline comment on the member line would be lost`,
    );
    error.name = 'JsoncCommentRemovalError';
    throw error;
  }
  let end = member.value.end;
  while (text[end] === ' ' || text[end] === '\t') end++;
  if (text[end] === ',') end++;
  const newline = text.indexOf('\n', end);
  return text.slice(0, lineStart) + text.slice(newline < 0 ? text.length : newline + 1);
}

// Like setJsoncMember, but inserts a value's source text verbatim instead of
// re-serializing it. renameJsoncProvider needs that: re-serializing would drop
// the comments inside a migrated provider section.
function setJsoncMemberRaw(text, object, key, rawValue) {
  const objectIndent = lineIndent(text, object.start);
  const memberIndent = `${objectIndent}  `;
  const entry = `${JSON.stringify(key)}: ${rawValue}`;
  if (object.members.length === 0) {
    return `${text.slice(0, object.start + 1)}\n${memberIndent}${entry}\n${objectIndent}${text.slice(object.start + 1)}`;
  }
  const at = object.members.at(-1).value.end;
  return `${text.slice(0, at)},\n${memberIndent}${entry}${text.slice(at)}`;
}

// Renames a provider member, moving its value's source text verbatim (#7333).
// No-ops when the provider is absent or the target id already exists, so a
// half-renamed config is never overwritten. Throws like removeJsoncPath when
// the source member carries an inline comment the move would lose.
function renameJsoncProvider(text, fromId, toId) {
  const root = parseJsoncTree(text);
  const providerObject = root.members.find(m => m.key === 'provider')?.value;
  if (!providerObject || providerObject.type !== 'object') return text;
  const from = providerObject.members.find(m => m.key === fromId);
  if (!from) return text;
  if (providerObject.members.some(m => m.key === toId)) return text;
  const rawValue = text.slice(from.value.start, from.value.end);
  const without = removeJsoncPath(text, ['provider', fromId]);
  const providerAfter = parseJsoncTree(without).members.find(m => m.key === 'provider');
  return setJsoncMemberRaw(without, providerAfter.value, toId, rawValue);
}

// Shape validation at the parse boundary (#2832, design #3068 Phase 2):
// parseJsonc proves the config parses; this proves the shape run() is about
// to dereference, before the first deref and before any side effect. A
// wrong-shaped entry produces one named, actionable error (config path +
// key path) instead of a raw TypeError that blocks every provider. A
// missing `provider` key is not an error: it means nothing to sync.
function validateConfigShape(config, configPath) {
  const shapeName = value =>
    value === null ? 'null' : Array.isArray(value) ? 'array' : typeof value;
  const isObject = value => value !== null && typeof value === 'object' && !Array.isArray(value);
  const fail = (keyPath, expected, value) => {
    throw new Error(
      `invalid config shape in ${configPath}: ${keyPath}: expected ${expected}, got ${shapeName(value)}`,
    );
  };
  if (!isObject(config)) fail('(root)', 'object', config);
  if (config.provider === undefined) return;
  if (!isObject(config.provider)) fail('provider', 'object', config.provider);
  for (const [name, prov] of Object.entries(config.provider)) {
    if (!isObject(prov)) fail(`provider.${name}`, 'object', prov);
    if (prov.models !== undefined && !isObject(prov.models)) {
      fail(`provider.${name}.models`, 'object', prov.models);
    }
  }
}

// -----------------------------------------------------------------------------
// Main Sync Runner
// -----------------------------------------------------------------------------
async function run() {
  const modelsDev = await fetchModelsDev();
  const agyModels = await discoverAntigravityModels();
  const policy = loadPolicy();

  if (!fs.existsSync(OPENCODE_CONFIG_PATH)) {
    console.error(`[sync-models] Error: ${OPENCODE_CONFIG_PATH} does not exist!`);
    process.exit(1);
  }

  const origConfigText = fs.readFileSync(OPENCODE_CONFIG_PATH, 'utf-8');
  let config;
  try {
    config = parseJsonc(origConfigText);
    validateConfigShape(config, OPENCODE_CONFIG_PATH);
  } catch (err) {
    console.error(`[sync-models] Error parsing ${OPENCODE_CONFIG_PATH}: ${err.message}`);
    process.exit(1);
  }

  let totalAdded = 0;
  let totalUpdated = 0;
  let totalRemoved = 0;
  let totalCreated = 0;
  let totalMigrated = 0;
  let totalSkipped = 0;
  let totalUnverified = 0;
  // Every change is recorded here and applied to the text at the end, so the
  // file keeps its comments and formatting.
  const edits = [];

  // #7333: resolve every sync-owned provider before syncing. A target section
  // can be missing while its catalog still sits under a pre-rename id; the
  // in-memory key move happens here, the matching text edit is applied below.
  // Resolution is never skipped silently: an absent section is reported (and,
  // when discovery has models, created) instead of falling through to an
  // "up to date" summary.
  const providerStatus = new Map();
  for (const targetProvider of ['_antigravity']) {
    const resolved = resolveProviderSection(config, targetProvider);
    providerStatus.set(targetProvider, resolved);
    if (resolved.status === 'migrated') {
      console.log(
        `[sync-models] ~ Renaming provider section ${resolved.migratedFrom} -> ${targetProvider} (renamed provider catalog)`,
      );
      edits.push({ renameProvider: [resolved.migratedFrom, targetProvider] });
      totalMigrated++;
    }
  }

  // 1. Sync _antigravity (the CLI-wire door; antigravity-agy &
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
        defineValue(grouped, baseId, {
          baseId,
          name: m.name.replace(/\s*\((Low|Medium|High|Extra-Low)\)$/i, ''),
          efforts: new Set(),
          variants: {}
        });
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

    for (const targetProvider of ['_antigravity']) {
      if (providerStatus.get(targetProvider).status === 'absent') {
        // #7333 item 2: discovery returned models for a provider the config
        // does not define yet — create the section instead of skipping it.
        const section = { models: {} };
        console.log(
          `[sync-models] + Creating provider section ${targetProvider} (discovery returned ${agyModels.length} models)`,
        );
        defineValue(config.provider, targetProvider, section);
        edits.push({ set: ['provider', targetProvider], value: section });
        totalCreated++;
      }
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
          defineValue(prov.models, baseId, modelEntry);
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
  } else {
    // #7333: with no discovered models the sync block never runs, so the
    // catalog could not be verified. Say so instead of letting the summary
    // claim everything is up to date. An absent section is the reported
    // defect (nothing was created); a present one is merely unverified.
    for (const [targetProvider, resolved] of providerStatus) {
      if (resolved.status === 'absent') {
        console.warn(
          `[sync-models] ! Skipped ${targetProvider}: section absent from ${OPENCODE_CONFIG_PATH} and model discovery returned nothing — the catalog was neither created nor verified (NOT up to date)`,
        );
        totalSkipped++;
      } else {
        console.warn(
          `[sync-models] ! Skipped ${targetProvider}: model discovery returned nothing, so the catalog was left unchanged and not verified (NOT up to date)`,
        );
        totalUnverified++;
      }
    }
  }

  // 2. openai-codex: retired 2026-11-03 -- the provider definition was
  // deleted (dead-provider retirement ordered by Drew) after its route
  // legs were re-seated onto openai-plan, so there is nothing to sync.

  // 3. Sync models.dev for cloud providers (openai; the google and
  // cloudflare-gateway syncs were retired 2026-11-03 with their provider definitions)
  if (modelsDev) {
    // google: retired 2026-11-03 -- the provider definition was deleted
    // (dead-provider retirement ordered by Drew), so there is nothing to sync.

    // Sync OpenAI
    if (config.provider?.openai && modelsDev.openai?.models) {
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
          defineValue(oProv.models, mId, entry);
          edits.push({ set: ['provider', 'openai', 'models', mId], value: entry });
          totalAdded++;
        }
      }
    }

    // cloudflare-gateway: retired 2026-11-03 with its provider definition.
  }

  console.log(
    `[sync-models] Summary: ${totalAdded} models added, ${totalUpdated} models updated, ` +
    `${totalRemoved} models removed, ${totalCreated} provider sections created, ` +
    `${totalMigrated} renamed, ${totalSkipped} skipped, ${totalUnverified} unverified.`,
  );

  // #7333: an absent target section means a catalog sync did not happen — the
  // daily service must not exit green for that. Exit 3 (not 1: the run did
  // what it could) leaves "skipped, needs attention" distinguishable from a
  // hard error. An unverified catalog (discovery down) is warned about above
  // but does not fail the run: that is a source-availability condition, not a
  // config defect.
  if (totalSkipped > 0) {
    console.error(
      `[sync-models] ${totalSkipped} sync target(s) skipped: this run did not maintain the model catalog (NOT up to date).`,
    );
    process.exitCode = 3;
  }

  const changed = totalAdded + totalUpdated + totalRemoved + totalCreated + totalMigrated;
  if (changed === 0) {
    if (totalSkipped === 0 && totalUnverified === 0) {
      console.log('[sync-models] All providers are already up-to-date!');
    }
    return;
  }

  let newText = origConfigText;
  for (const edit of edits) {
    newText = edit.remove
      ? removeJsoncPath(newText, edit.remove)
      : edit.renameProvider
        ? renameJsoncProvider(newText, edit.renameProvider[0], edit.renameProvider[1])
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
