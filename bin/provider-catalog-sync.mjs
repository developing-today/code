#!/usr/bin/env node
// provider-catalog-sync.mjs — automatic provider-catalog updates (issue #1297).
//
// Enumerates each provider's authoritative model list, diffs it against the
// catalog in .opencode/opencode.jsonc, and classifies the drift:
//
//   additions_validated  In the provider's OWN live list, missing from the
//                        config. The only class --apply will ever add.
//   additions_unverified In an upstream catalog (models.dev) only. Reported,
//                        NEVER auto-added: a listing is not proof a door
//                        serves the model (Drew's validation rule, 2026-10-27).
//   vanishings           In the config, absent from the provider's live list.
//                        Flagged only, NEVER auto-removed — the config carries
//                        deliberate extras (e.g. commandcode's
//                        reflection/beam:free, which the proxy serves without
//                        advertising) and routes may still reference an id.
//   unverifiable         The provider cannot be enumerated (e.g. opendesign's
//                        AMR /v1/models is API-key gated). Reported as such;
//                        never guessed.
//
// Modes:
//   (default)  dry-run report. Exit 0 = no validated additions, 3 = validated
//              additions found, 2 = hard error. A provider that fails to
//              enumerate degrades to a per-provider note, never a failed run.
//   --apply    additionally write validated additions into the config file
//              (catalog-only edits, sibling-mirrored entry shapes), print a
//              PR body draft, and with --open-pr also commit on a dedicated
//              chore/catalog-sync-<date> branch (created or reused for the
//              day, never the default branch), push it with the upstream
//              set, and open the PR from that branch via gh.
//
// This is the PR-based successor to bin/sync-models.mjs, which rewrites the
// deployed config in place (and auto-removes). Per Drew's policy the catalog
// lists ALL models a provider allows; gating lives in disabled_providers and
// route membership, never in the catalog — so this tool only ever ADDS,
// through a reviewable PR.
//
// #4544 split: the tracked configured default no longer carries the model
// catalogs sync-models owns (_antigravity, google,
// openai, cloudflare-gateway) — those live in the runtime overlay
// (~/.local/state/opencode/opencode-runtime.jsonc). This tool diffs the
// merged effective config (default + overlay; --runtime-config) and skips
// the sync-owned providers on --apply; sync-models owns their writes.
//
// Config resolution (#4588): this tool's target is the tracked configured
// default — an explicit --config wins, then OPENCODE_CATALOG_SYNC_CONFIG,
// then <repo>/.opencode/opencode.jsonc. OPENCODE_CONFIG_PATH is never the
// target: it is sync-models' variable and since #4544 it names the runtime
// overlay, which this tool only reads (via --runtime-config) to compute the
// merged effective config.
//
// Sources: laptop proxies are enumerated over HTTP on 127.0.0.1 (run this on
// the laptop, or pass --source-dir with <port>.json snapshots for an offline
// reproducible run). models.dev is a cross-check for every provider and the
// only source for providers whose own endpoint needs a key we do not use
// here; their additions stay unverified by construction.

import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

// #2953 (define, don't assign): store an externally-spelled key with
// define semantics, so "__proto__" becomes an own property instead of
// invoking the Object.prototype setter and "constructor"/"toString"
// shadow like any other key.
const defineValue = (obj, key, value) => {
  Object.defineProperty(obj, key, { value, writable: true, enumerable: true, configurable: true });
  return value;
};


// ---------------------------------------------------------------------------
// Provider source table. `modelsDev` is the provider id in models.dev's
// api.json used as cross-check (null = no meaningful cross-check).
// ---------------------------------------------------------------------------
export const PROVIDER_SOURCES = {
  // No claude-code / commandcode / antigravity-agy / antigravity-ls
  // / openai-codex entries: their CLI proxies were removed in the
  // 2026-11-06 proxy-infra decommission (claude-code and
  // commandcode were repointed to native fork providers on
  // 2026-11-05; the antigravity doors retired 2026-10-30; the
  // openai-codex provider was deleted 2026-11-03 and its door
  // retired with the rest). Their catalogs are native-owned now,
  // so they report 'no-source-configured'.
  // One deliberately synthetic proxy-kind source: 'fixture-proxy'
  // is the enumeration machinery's worked example, kept after the
  // last real proxy source (the codex-proxy 'chatgpt' entry) was
  // removed 2026-11-06 on Drew's ruling (kill record:
  // developing-today-forks/opencode#1825). The CLI control fixtures
  // (#3449/#4588) exercise the validated-addition path through this
  // entry's source-dir snapshot, and no live proxy-kind source
  // remains. No live config names a fixture-proxy provider, so the
  // entry is dormant; a run that ever names one reports the
  // tolerated enumeration failure against the fixture port :18799
  // (nothing listens there by design) and falls back to
  // cross-check only.
  'fixture-proxy': { kind: 'proxy', port: 18799, modelsDev: 'openai' },
  deepseek: { kind: 'crosscheck', modelsDev: 'deepseek' },
  groq: { kind: 'crosscheck', modelsDev: 'groq' },
  meta: { kind: 'crosscheck', modelsDev: 'meta' },
  'cline-pass': { kind: 'crosscheck', modelsDev: 'cline-pass' },
  google: { kind: 'crosscheck', modelsDev: 'google' },
  openai: { kind: 'crosscheck', modelsDev: 'openai' },
  'cloudflare-ai-gateway': { kind: 'crosscheck', modelsDev: 'cloudflare-ai-gateway' },
  opendesign: {
    kind: 'unverifiable',
    note: 'AMR /v1/models is API-key gated (401 without a key); no models.dev catalog.',
  },
  'openai.alternate': {
    kind: 'unverifiable',
    note: 'No catalog block of its own; resolves through the fork built-in catalog (models.dev feed).',
  },
  'opencode-go.drewrypope': {
    kind: 'unverifiable',
    note: 'No catalog block of its own; resolves through the fork built-in catalog (models.dev feed).',
  },
  'opencode-go.popesdrew': {
    kind: 'unverifiable',
    note: 'No catalog block of its own; resolves through the fork built-in catalog (models.dev feed).',
  },
};

// ---------------------------------------------------------------------------
// JSONC reading: strings-aware comment strip + trailing-comma fix (same
// approach as the provider-sync audit tooling), producing plain JSON.
// ---------------------------------------------------------------------------
export function stripJsonc(text) {
  const out = [];
  let i = 0;
  let instr = false;
  let esc = false;
  while (i < text.length) {
    const c = text[i];
    if (instr) {
      out.push(c);
      if (esc) esc = false;
      else if (c === '\\') esc = true;
      else if (c === '"') instr = false;
      i += 1;
      continue;
    }
    if (c === '"') {
      instr = true;
      out.push(c);
      i += 1;
      continue;
    }
    if (c === '/' && text[i + 1] === '/') {
      while (i < text.length && text[i] !== '\n') i += 1;
      continue;
    }
    if (c === '/' && text[i + 1] === '*') {
      i += 2;
      while (i + 1 < text.length && !(text[i] === '*' && text[i + 1] === '/')) i += 1;
      i += 2;
      continue;
    }
    out.push(c);
    i += 1;
  }
  const stripped = out.join('');
  // String-aware trailing-comma removal: a ',' followed by only whitespace
  // then '}' or ']' is structural, but the same sequence inside a string
  // literal is content and must survive verbatim (#3430).
  const res = [];
  let j = 0;
  let inStr = false;
  let e = false;
  while (j < stripped.length) {
    const ch = stripped[j];
    if (inStr) {
      res.push(ch);
      if (e) e = false;
      else if (ch === '\\') e = true;
      else if (ch === '"') inStr = false;
      j += 1;
      continue;
    }
    if (ch === '"') {
      inStr = true;
      res.push(ch);
      j += 1;
      continue;
    }
    if (ch === ',') {
      let k = j + 1;
      while (k < stripped.length && /\s/.test(stripped[k])) k += 1;
      if (stripped[k] === '}' || stripped[k] === ']') {
        j += 1;
        continue;
      }
      res.push(ch);
      j += 1;
      continue;
    }
    res.push(ch);
    j += 1;
  }
  return res.join('');
}

export function parseJsonc(text) {
  return JSON.parse(stripJsonc(text));
}

// Same-length copy of the text with comments replaced by spaces (strings
// preserved), so scanner offsets stay valid against the original text.
function blankComments(text) {
  const arr = text.split('');
  let i = 0;
  let instr = false;
  let esc = false;
  while (i < arr.length) {
    const c = arr[i];
    if (instr) {
      if (esc) esc = false;
      else if (c === '\\') esc = true;
      else if (c === '"') instr = false;
      i += 1;
      continue;
    }
    if (c === '"') {
      instr = true;
      i += 1;
      continue;
    }
    if (c === '/' && arr[i + 1] === '/') {
      while (i < arr.length && arr[i] !== '\n') {
        arr[i] = ' ';
        i += 1;
      }
      continue;
    }
    if (c === '/' && arr[i + 1] === '*') {
      arr[i] = ' ';
      arr[i + 1] = ' ';
      i += 2;
      while (i + 1 < arr.length && !(arr[i] === '*' && arr[i + 1] === '/')) {
        if (arr[i] !== '\n') arr[i] = ' ';
        i += 1;
      }
      if (i + 1 < arr.length) {
        arr[i] = ' ';
        arr[i + 1] = ' ';
        i += 2;
      }
      continue;
    }
    i += 1;
  }
  return arr.join('');
}

// Minimal JSONC object scanner over comment-blanked text. Given the index of
// an opening brace, returns the member list [{key, valueStart, valueEnd}] and
// the index of the matching closing brace. Values are located, not parsed.
export function scanObject(text, openIdx) {
  const members = [];
  let i = openIdx + 1;
  const skipWs = () => {
    while (i < text.length && /[\s,]/.test(text[i])) i += 1;
  };
  const readString = () => {
    let j = i + 1;
    let esc = false;
    while (j < text.length) {
      const c = text[j];
      if (esc) esc = false;
      else if (c === '\\') esc = true;
      else if (c === '"') break;
      j += 1;
    }
    const s = JSON.parse(text.slice(i, j + 1));
    i = j + 1;
    return s;
  };
  const skipValue = () => {
    const start = i;
    const c = text[i];
    if (c === '{' || c === '[') {
      const open = c;
      const close = c === '{' ? '}' : ']';
      let depth = 0;
      let instr = false;
      let esc = false;
      while (i < text.length) {
        const ch = text[i];
        if (instr) {
          if (esc) esc = false;
          else if (ch === '\\') esc = true;
          else if (ch === '"') instr = false;
        } else if (ch === '"') instr = true;
        else if (ch === open) depth += 1;
        else if (ch === close) {
          depth -= 1;
          if (depth === 0) {
            i += 1;
            break;
          }
        }
        i += 1;
      }
      return { start, end: i };
    }
    if (c === '"') {
      const s = i;
      readString();
      return { start: s, end: i };
    }
    while (i < text.length && !/[\s,}\]]/.test(text[i])) i += 1;
    return { start, end: i };
  };
  skipWs();
  while (i < text.length && text[i] !== '}') {
    if (text[i] !== '"') throw new Error(`scanObject: expected key at ${i}`);
    const key = readString();
    skipWs();
    if (text[i] !== ':') throw new Error(`scanObject: expected ':' at ${i}`);
    i += 1;
    skipWs();
    const v = skipValue();
    members.push({ key, valueStart: v.start, valueEnd: v.end });
    skipWs();
  }
  if (i >= text.length) throw new Error('scanObject: unterminated object');
  return { members, closeIdx: i };
}

// Locate provider.<id>.models in the raw config text. Returns
// { modelsOpen, modelsClose, entryIndent } or null when the provider or its
// models object is absent.
export function findModelsSpan(text, providerId) {
  const clean = blankComments(text);
  const root = scanObject(clean, clean.indexOf('{'));
  const provMember = root.members.find((m) => m.key === 'provider');
  if (!provMember) return null;
  const provObj = scanObject(clean, provMember.valueStart);
  const block = provObj.members.find((m) => m.key === providerId);
  if (!block) return null;
  const blockObj = scanObject(clean, block.valueStart);
  const models = blockObj.members.find((m) => m.key === 'models');
  if (!models) return null;
  const modelsObj = scanObject(clean, models.valueStart);
  let entryIndent = null;
  if (modelsObj.members.length > 0) {
    const firstKeyIdx = models.valueStart + clean.slice(models.valueStart, modelsObj.members[0].valueStart).indexOf('"');
    const lineStart = clean.lastIndexOf('\n', firstKeyIdx) + 1;
    entryIndent = clean.slice(lineStart, firstKeyIdx);
  }
  return {
    modelsOpen: models.valueStart,
    modelsClose: modelsObj.closeIdx,
    entryIndent: entryIndent ?? '      ',
    memberCount: modelsObj.members.length,
  };
}

// Insert one model entry into provider.<id>.models, preserving the file's
// formatting. Returns the new text. Throws if the span cannot be found.
export function insertModelEntry(text, providerId, modelId, entry) {
  const span = findModelsSpan(text, providerId);
  if (!span) throw new Error(`insertModelEntry: no models object for provider "${providerId}"`);
  const indent = span.entryIndent;
  // JSON.stringify already indents nested levels by 2; prefixing each
  // continuation line with the entry indent keeps the file's 2-space steps.
  const entryJson = JSON.stringify(entry, null, 2)
    .split('\n')
    .map((line, idx) => (idx === 0 ? line : indent + line))
    .join('\n');
  const rendered = `${indent}"${modelId}": ${entryJson}`;
  if (span.memberCount === 0) {
    return text.slice(0, span.modelsOpen + 1) + '\n' + rendered + '\n' + indent.slice(0, -2) + text.slice(span.modelsClose);
  }
  // Insert before the closing brace; the preceding entry needs a comma.
  let j = span.modelsClose - 1;
  while (j > span.modelsOpen && /\s/.test(text[j])) j -= 1;
  const needsComma = text[j] !== ',';
  const insertion = (needsComma ? ',' : '') + '\n' + rendered;
  return text.slice(0, j + 1) + insertion + text.slice(j + 1);
}

// ---------------------------------------------------------------------------
// Enumeration + classification
// ---------------------------------------------------------------------------

// Proxy lists are compared RAW, exactly as served: the fleet catalog mirrors
// the proxies' served lists verbatim (the #1304 sync convention), including
// rung-expanded ids (gpt-6.1-sol-xhigh) where a proxy emits them as separate
// models and base ids with variant ladders where it does not. Collapsing
// rungs here would manufacture false vanishings out of that mixed reality.
export function classify({ configIds, liveIds, crosscheckIds }) {
  const config = new Set(configIds);
  const result = {
    additions_validated: [],
    additions_unverified: [],
    vanishings: [],
    vanishings_basis: null,
    unknown_models: [],
  };
  const cross = crosscheckIds ? new Set(crosscheckIds) : null;
  if (liveIds) {
    const live = new Set(liveIds);
    result.additions_validated = [...live].filter((id) => !config.has(id)).sort();
    result.vanishings = [...config].filter((id) => !live.has(id)).sort();
    result.vanishings_basis = 'live';
  }
  if (cross) {
    const unv = [...cross].filter((id) => !config.has(id) && !result.additions_validated.includes(id)).sort();
    result.additions_unverified = unv;
    if (!liveIds) {
      result.vanishings = [...config].filter((id) => !cross.has(id)).sort();
      result.vanishings_basis = 'crosscheck';
    }
  }
  const known = cross ?? new Set();
  result.unknown_models = [...result.additions_validated, ...result.additions_unverified]
    .filter((id) => !known.has(id))
    .sort();
  return result;
}

// ---------------------------------------------------------------------------
// Entry generation (sibling-mirrored)
// ---------------------------------------------------------------------------
const ACRONYMS = { gpt: 'GPT', glm: 'GLM', ai: 'AI', llm: 'LLM', vl: 'VL', api: 'API' };
const BRANDS = { deepseek: 'DeepSeek', openai: 'OpenAI', qwen: 'Qwen' };

export function prettifyModelName(modelId) {
  const parts = modelId.split('/');
  // Namespaced ids (vendor/model) drop the vendor segment when the model
  // name already starts with it: deepseek/deepseek-v4-pro -> DeepSeek v4 Pro.
  if (parts.length > 1 && parts[parts.length - 1].toLowerCase().startsWith(parts[parts.length - 2].toLowerCase())) {
    parts.splice(parts.length - 2, 1);
  }
  return parts
    .map((part) =>
      part
        .split('-')
        .map((tok) => {
          if (!tok) return tok;
          if (BRANDS[tok.toLowerCase()]) return BRANDS[tok.toLowerCase()];
          if (ACRONYMS[tok.toLowerCase()]) return ACRONYMS[tok.toLowerCase()];
          if (/^v\d/i.test(tok)) return tok.toLowerCase();
          if (/^\d+$/.test(tok)) return tok;
          return tok.charAt(0).toUpperCase() + tok.slice(1);
        })
        .join(' '),
    )
    .join(' ')
    .replace(/(\d) (\d)/g, '$1.$2');
}

function commonSuffix(names) {
  const counts = {};
  for (const n of names) {
    const m = n.match(/(\([^)]*\))\s*$/);
    if (m) defineValue(counts, m[1], (Object.hasOwn(counts, m[1]) ? counts[m[1]] : 0) + 1);
  }
  const best = Object.entries(counts).sort((a, b) => b[1] - a[1])[0];
  return best ? best[0] : null;
}

function longestCommonPrefixSibling(modelId, siblingIds) {
  let best = null;
  let bestLen = -1;
  for (const id of siblingIds) {
    let k = 0;
    while (k < id.length && k < modelId.length && id[k] === modelId[k]) k += 1;
    if (k > bestLen) {
      bestLen = k;
      best = id;
    }
  }
  return best;
}

// Build a catalog entry for a new model, mirroring the block's siblings:
// name-only blocks get a name-only entry (plus an "id" field where that is
// the block's style); richer blocks clone the nearest family sibling's
// variants and prefer models.dev limits when available.
export function makeEntry(providerId, modelId, siblings, modelsDevEntry) {
  const ids = Object.keys(siblings);
  const entries = Object.values(siblings);
  const suffix = commonSuffix(entries.map((e) => e?.name).filter(Boolean));
  const name = prettifyModelName(modelId) + (suffix ? ` ${suffix}` : '');
  // A cloneable sibling carries a variants OBJECT. A present-but-null (or
  // otherwise non-object) variants value is not cloneable: the branch gate
  // and the clone-source filter below must agree on this, or a block whose
  // only variant-carrying entries are nulled takes the clone branch with
  // zero candidates and dies in the JSON round-trip on siblings[null] (#6907).
  const hasCloneableVariants = (e) =>
    e && typeof e === 'object' && e.variants !== null && typeof e.variants === 'object';
  const withVariants = entries.filter(hasCloneableVariants);
  const withId = entries.filter((e) => e && typeof e === 'object' && 'id' in e);
  const entry = {};
  if (withId.length > entries.length / 2) entry.id = `${providerId}/${modelId}`;
  entry.name = name;
  if (withVariants.length > 0) {
    const sibId = longestCommonPrefixSibling(modelId, ids.filter((id) => hasCloneableVariants(siblings[id])));
    const clone = JSON.parse(JSON.stringify(siblings[sibId]));
    if (modelsDevEntry?.limit?.context) {
      entry.limit = {
        context: modelsDevEntry.limit.context,
        ...(modelsDevEntry.limit.output ? { output: modelsDevEntry.limit.output } : {}),
      };
    } else if (clone.limit) entry.limit = clone.limit;
    entry.variants = clone.variants;
    entry._clonedFrom = sibId;
  } else if (modelsDevEntry?.limit?.context) {
    entry.limit = {
      context: modelsDevEntry.limit.context,
      ...(modelsDevEntry.limit.output ? { output: modelsDevEntry.limit.output } : {}),
    };
  }
  return entry;
}

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------
async function fetchJson(url, timeoutMs = 15000) {
  const res = await fetch(url, { signal: AbortSignal.timeout(timeoutMs) });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

function modelIdsFromProxyResponse(data) {
  if (!data || typeof data !== 'object' || Array.isArray(data)) {
    throw new Error('response body must be a non-null object');
  }
  if (!Object.prototype.hasOwnProperty.call(data, 'data')) {
    throw new Error('response body is missing data');
  }
  if (!Array.isArray(data.data)) {
    throw new Error('response data must be an array');
  }
  return data.data.map((model, index) => {
    if (!model || typeof model !== 'object' || Array.isArray(model)) {
      throw new Error(`response data[${index}] must be an object`);
    }
    if (typeof model.id !== 'string' || model.id.trim().length === 0) {
      throw new Error(`response data[${index}].id must be a non-empty string`);
    }
    return model.id;
  });
}

async function enumerateProxy(port, sourceDir) {
  if (sourceDir) {
    const file = path.join(sourceDir, `${port}.json`);
    const data = JSON.parse(fs.readFileSync(file, 'utf8'));
    return modelIdsFromProxyResponse(data);
  }
  const data = await fetchJson(`http://127.0.0.1:${port}/v1/models`, 10000);
  return modelIdsFromProxyResponse(data);
}

async function loadModelsDev(spec) {
  if (!spec) return null;
  if (fs.existsSync(spec)) return JSON.parse(fs.readFileSync(spec, 'utf8'));
  return fetchJson(spec, 30000);
}

// ---------------------------------------------------------------------------
// Run
// ---------------------------------------------------------------------------
// Providers whose model catalogs bin/sync-models.mjs owns in the runtime
// overlay (#4544). Catalog-sync diffs them (via the merged effective config)
// but never writes them into the tracked configured default on --apply.
export const SYNC_OWNED_PROVIDERS = new Set([
  '_antigravity',
  'google',
  'openai',
  'cloudflare-gateway',
]);

// Deep-merge plain objects (overlay wins on scalars) — the same layering
// the OpenCode server applies to OPENCODE_CONFIG.
export function deepMergeProviders(base, over) {
  const isObj = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);
  if (!isObj(base) || !isObj(over)) return over;
  const out = { ...base };
  const hasOwn = (obj, key) => Object.prototype.hasOwnProperty.call(obj, key);
  for (const k of Object.keys(over)) {
    // #4957 (design #2953): the overlay is an external file, so a key it
    // spells must become an own data property at every depth. A plain
    // `out[k] = ...` is a [[Set]] write: for `__proto__` that invokes the
    // inherited accessor and moves the prototype instead of storing the
    // value. Define the leaf explicitly. The own-property check also keeps
    // a missing base key from resolving through the prototype chain
    // (base["__proto__"] would otherwise be Object.prototype itself).
    Object.defineProperty(out, k, {
      value: deepMergeProviders(hasOwn(base, k) ? base[k] : undefined, over[k]),
      enumerable: true,
      writable: true,
      configurable: true,
    });
  }
  return out;
}

export async function runSync({ configPath, modelsDevSpec, sourceDir, runtimeConfigPath }) {
  const configText = fs.readFileSync(configPath, 'utf8');
  const config = parseJsonc(configText);
  // #4544: diff against the merged effective config (tracked default +
  // runtime overlay) so the sync-owned providers still enumerate and their
  // vanishings still surface exactly as before the split.
  let runtimeOverlay = null;
  if (runtimeConfigPath && fs.existsSync(runtimeConfigPath)) {
    try {
      const overlay = parseJsonc(fs.readFileSync(runtimeConfigPath, 'utf8'));
      if (overlay && typeof overlay === 'object' && overlay.provider) {
        config.provider = deepMergeProviders(config.provider ?? {}, overlay.provider);
        runtimeOverlay = runtimeConfigPath;
      }
    } catch {
      runtimeOverlay = null;
    }
  }
  const providers = config.provider ?? {};
  let modelsDev = null;
  let modelsDevError = null;
  try {
    modelsDev = await loadModelsDev(modelsDevSpec);
  } catch (err) {
    modelsDevError = String(err?.message ?? err);
  }
  const report = {
    generated_at: new Date().toISOString(),
    config: configPath,
    runtime_overlay: runtimeOverlay,
    models_dev: modelsDev ? 'loaded' : `unavailable (${modelsDevError})`,
    providers: {},
    totals: { additions_validated: 0, additions_unverified: 0, vanishings: 0, unverifiable: 0 },
  };
  // Cache proxy enumerations by port: providers sharing one door share one fetch.
  const proxyCache = new Map();
  for (const [providerId, block] of Object.entries(providers)) {
    const configIds = Object.keys(block?.models ?? {});
    const src = PROVIDER_SOURCES[providerId];
    const entry = { config_count: configIds.length };
    if (!src) {
      entry.status = 'no-source-configured';
      defineValue(report.providers, providerId, entry);
      continue;
    }
    entry.source = src.kind;
    // Some models.dev providers namespace their model ids with the provider
    // slug (cline-pass ships "cline-pass/glm-5.3" for the config's "glm-5.3").
    // Strip that one prefix so the cross-check compares like with like.
    const crossIds =
      src.modelsDev && modelsDev?.[src.modelsDev]?.models
        ? Object.keys(modelsDev[src.modelsDev].models).map((id) =>
            id.startsWith(src.modelsDev + '/') ? id.slice(src.modelsDev.length + 1) : id,
          )
        : null;
    entry.crosscheck_count = crossIds ? crossIds.length : null;
    if (src.kind === 'unverifiable') {
      entry.status = 'unverifiable';
      entry.note = src.note;
      report.totals.unverifiable += 1;
      defineValue(report.providers, providerId, entry);
      continue;
    }
    if (src.kind === 'proxy') {
      try {
        if (!proxyCache.has(src.port)) proxyCache.set(src.port, await enumerateProxy(src.port, sourceDir));
        const rawIds = proxyCache.get(src.port);
        entry.live_count = rawIds.length;
        Object.assign(entry, classify({ configIds, liveIds: rawIds, crosscheckIds: crossIds }));
      } catch (err) {
        entry.status = 'not-enumerable';
        entry.note = `proxy :${src.port} enumeration failed: ${err?.message ?? err}`;
        if (crossIds) {
          Object.assign(entry, classify({ configIds, liveIds: null, crosscheckIds: crossIds }));
          entry.note += '; fell back to cross-check only (additions stay unverified)';
        }
      }
    } else {
      // crosscheck-only: the provider's own endpoint needs credentials this
      // job does not use; models.dev diffs are reported, never applied.
      if (!crossIds) {
        entry.status = 'not-enumerable';
        entry.note = 'models.dev unavailable or has no catalog for this provider';
      } else {
        entry.live_count = null;
        Object.assign(entry, classify({ configIds, liveIds: null, crosscheckIds: crossIds }));
      }
    }
    for (const k of ['additions_validated', 'additions_unverified', 'vanishings']) {
      report.totals[k] += entry[k]?.length ?? 0;
    }
    defineValue(report.providers, providerId, entry);
  }
  return { report, configText, config };
}

export function renderHuman(report) {
  const lines = [];
  lines.push(`provider-catalog-sync report — ${report.generated_at}`);
  lines.push(`config: ${report.config} | models.dev: ${report.models_dev}`);
  for (const [pid, e] of Object.entries(report.providers)) {
    const bits = [`${pid}: source=${e.source ?? 'none'} config=${e.config_count}`];
    if (e.live_count != null) bits.push(`live=${e.live_count}`);
    if (e.crosscheck_count != null) bits.push(`crosscheck=${e.crosscheck_count}`);
    if (e.status) bits.push(`status=${e.status}`);
    lines.push(bits.join(' '));
    if (e.note) lines.push(`  note: ${e.note}`);
    for (const id of e.additions_validated ?? []) lines.push(`  + VALIDATED ADDITION: ${id}`);
    for (const id of e.additions_unverified ?? []) lines.push(`  + unverified (catalog only): ${id}`);
    const vBasis = e.vanishings_basis === 'crosscheck' ? ' (vs models.dev catalog)' : '';
    for (const id of e.vanishings ?? []) lines.push(`  - vanishing (flag only)${vBasis}: ${id}`);
    for (const id of e.unknown_models ?? []) lines.push(`  ? unknown to models.dev (research trail): ${id}`);
  }
  const t = report.totals;
  lines.push(
    `totals: ${t.additions_validated} validated additions, ${t.additions_unverified} unverified, ${t.vanishings} vanishings flagged, ${t.unverifiable} unverifiable providers`,
  );
  return lines.join('\n');
}

export function renderPrBody(report, addedByProvider) {
  const lines = [];
  lines.push('## Automatic provider-catalog update (issue #1297)');
  lines.push('');
  lines.push(`Enumerated ${report.generated_at}; sources: each provider's own live \`/v1/models\` (laptop proxies) with models.dev as cross-check.`);
  lines.push('');
  lines.push('### Added (validated: present in the provider\'s own live list)');
  for (const [pid, ids] of Object.entries(addedByProvider)) {
    for (const id of ids) lines.push(`- \`${pid}/${id}\``);
  }
  lines.push('');
  lines.push('Entry shapes mirror sibling entries in each block; variants are cloned from the nearest family sibling and limits come from models.dev where available — both are called out per entry in the run report.');
  const flagged = Object.entries(report.providers).filter(([, e]) => (e.vanishings ?? []).length > 0);
  if (flagged.length) {
    lines.push('');
    lines.push('### Vanishings (flagged only — never auto-removed)');
    for (const [pid, e] of flagged) for (const id of e.vanishings) lines.push(`- \`${pid}/${id}\` is in the config but not in the provider's current list.`);
  }
  const unv = Object.entries(report.providers).filter(([, e]) => (e.additions_unverified ?? []).length > 0);
  if (unv.length) {
    lines.push('');
    lines.push('### Catalog-only sightings (NOT added — unverified)');
    for (const [pid, e] of unv)
      for (const id of e.additions_unverified) lines.push(`- \`${pid}/${id}\` appears in models.dev only; no serving door lists it yet.`);
  }
  lines.push('');
  lines.push('Catalog-only change: no routes, no `disabled_providers`, no pool membership touched. Relates to #1297.');
  return lines.join('\n');
}

// Apply validated additions to the config text. Returns { text, added }.
export function applyAdditions(configText, config, report, modelsDev) {
  let text = configText;
  const added = new Map();
  for (const [pid, e] of Object.entries(report.providers)) {
    const ids = e.additions_validated ?? [];
    if (!ids.length) continue;
    const src = PROVIDER_SOURCES[pid];
    const mdModels = src?.modelsDev ? modelsDev?.[src.modelsDev]?.models : null;
    for (const id of ids) {
      const entry = makeEntry(pid, id, config.provider[pid].models ?? {}, mdModels?.[id]);
      const clonedFrom = entry._clonedFrom;
      delete entry._clonedFrom;
      text = insertModelEntry(text, pid, id, entry);
      if (!added.has(pid)) added.set(pid, []);
      added.get(pid).push(id);
      if (clonedFrom) console.error(`[catalog-sync] ${pid}/${id}: variants cloned from sibling ${clonedFrom}`);
    }
  }
  return { text, added: Object.fromEntries(added) };
}

// Catalog-only guard: parse before/after and prove nothing outside the
// added catalog entries changed (routes, disabled_providers, other blocks).
export function assertCatalogOnly(before, after, added) {
  const b = parseJsonc(before);
  const a = parseJsonc(after);
  const strip = (cfg) => {
    const c = JSON.parse(JSON.stringify(cfg));
    for (const [pid, ids] of Object.entries(added)) {
      for (const id of ids) delete c.provider[pid].models[id];
    }
    return c;
  };
  if (JSON.stringify(strip(b)) !== JSON.stringify(strip(a))) {
    throw new Error('catalog-only guard failed: non-catalog content changed');
  }
}

// ---------------------------------------------------------------------------
// --open-pr branch contract (issue #1754). --open-pr exists to open a PR, so
// it must never commit on or push the repository default branch: the sync
// commits land on a dedicated chore/catalog-sync-<date> branch (the same UTC
// date the commit message carries), created before the commit — or checked
// out when it already exists, so a same-day rerun updates that day's PR.
// ---------------------------------------------------------------------------
export function catalogSyncBranch(date) {
  return `chore/catalog-sync-${date}`;
}

// Defence in depth: after the branch step the checked-out branch must not
// still be the default branch. Throws (aborting before any commit) when it
// is — a silent commit on the default branch is exactly the #1754 failure.
export function assertNotDefaultBranch(currentBranch, defaultBranch) {
  if (currentBranch === defaultBranch) {
    throw new Error(
      `--open-pr refuses to commit on the default branch (${defaultBranch}): ` +
        'the catalog-sync branch step did not move the checkout — aborting before any commit',
    );
  }
}

// The full git/gh command plan for --open-pr, as data, so tests can pin the
// contract (branch step BEFORE the commit, --set-upstream push of the
// dedicated branch, gh pr create --head) without executing anything.
// Step 0 is the branch step; main() runs it, then asserts the checkout is
// off the default branch, then runs the remaining steps.
export function openPrCommands({ branch, branchExists, date, bodyFile }) {
  return [
    branchExists ? ['git', 'checkout', branch] : ['git', 'checkout', '-b', branch],
    ['git', 'add', '.opencode/opencode.jsonc'],
    [
      'git',
      'commit',
      '-m',
      `chore(opencode): catalog sync ${date} — add validated provider models (#1297)`,
    ],
    ['git', 'push', '--set-upstream', 'origin', branch],
    [
      'gh',
      'pr',
      'create',
      '--title',
      `chore(opencode): catalog sync ${date} (#1297)`,
      '--body-file',
      bodyFile,
      '--head',
      branch,
    ],
  ];
}

// ---------------------------------------------------------------------------
// Strict argv parsing (issue #3449). Before this, opt() returned argv[idx+1]
// unchecked and unknown arguments were never rejected: flag tokens were
// consumed as values ('--config --apply' opened a file named '--apply'), a
// token could be consumed twice ('--models-dev --apply' both set the
// models.dev spec AND enabled apply), a trailing value flag crashed with a
// raw TypeError that never named the flag, and a typo'd '--aply' silently
// degraded an apply run into a dry run. Validation runs before any file
// read, fetch, or write; a failure is a usage error (exit 2, this script's
// hard-error code, matching the sibling usage-error pattern from
// #3419/#3295) that names the offending flag.
// ---------------------------------------------------------------------------
export const VALUE_OPTIONS = ['--config', '--models-dev', '--source-dir', '--repo', '--runtime-config'];
export const FLAG_OPTIONS = ['--apply', '--open-pr', '--json', '--help'];
export const USAGE =
  'usage: provider-catalog-sync [--config <path>] [--models-dev <url|path>] ' +
  '[--source-dir <dir>] [--repo <dir>] [--runtime-config <path>] [--apply] [--open-pr] [--json]';

export class UsageError extends Error {}

export function parseArgs(argv) {
  const options = {};
  const flags = new Set();
  for (let i = 0; i < argv.length; i += 1) {
    const token = argv[i];
    if (FLAG_OPTIONS.includes(token)) {
      flags.add(token);
      continue;
    }
    if (VALUE_OPTIONS.includes(token)) {
      if (Object.hasOwn(options, token)) {
        throw new UsageError(`${token} was given more than once\n${USAGE}`);
      }
      const value = argv[i + 1];
      if (value === undefined || value.startsWith('--')) {
        throw new UsageError(
          `${token} requires a value, but ${
            value === undefined ? 'no value followed it' : `the next token is the flag ${value}`
          }\n${USAGE}`,
        );
      }
      options[token] = value;
      i += 1;
      continue;
    }
    throw new UsageError(`unrecognised argument: ${token}\n${USAGE}`);
  }
  return { options, flags };
}

// Resolution order for the catalog target (#4588): explicit --config,
// then the catalog-sync-specific OPENCODE_CATALOG_SYNC_CONFIG, then the
// tracked configured default. OPENCODE_CONFIG_PATH is deliberately not
// consulted — since #4544 it names the runtime overlay (sync-models' file).
export function resolveConfigPath({ options, env, root }) {
  return (
    options['--config'] ??
    env.OPENCODE_CATALOG_SYNC_CONFIG ??
    path.join(root, '.opencode', 'opencode.jsonc')
  );
}

async function main(argv) {
  const { options, flags } = parseArgs(argv);
  if (flags.has('--help')) {
    console.log(USAGE);
    return 0;
  }
  const opt = (name, dflt) => options[name] ?? dflt;
  const root = path.resolve(path.dirname(new URL(import.meta.url).pathname), '..');
  const configPath = resolveConfigPath({ options, env: process.env, root });
  const runtimeConfigPath = opt(
    '--runtime-config',
    process.env.OPENCODE_RUNTIME_CONFIG ||
      path.join(
        process.env.HOME || '/home/user',
        '.local/state/opencode/opencode-runtime.jsonc',
      ),
  );
  const modelsDevSpec = opt('--models-dev', 'https://models.dev/api.json');
  const sourceDir = opt('--source-dir', null);
  const apply = flags.has('--apply');
  const openPr = flags.has('--open-pr');

  const { report, configText, config } = await runSync({
    configPath,
    modelsDevSpec,
    sourceDir,
    runtimeConfigPath,
  });

  if (!apply) {
    if (flags.has('--json')) console.log(JSON.stringify(report, null, 2));
    else console.log(renderHuman(report));
    return report.totals.additions_validated > 0 ? 3 : 0;
  }

  if (report.totals.additions_validated === 0) {
    console.log('[catalog-sync] no validated additions — nothing to apply (silent run).');
    return 0;
  }
  let modelsDev = null;
  try {
    modelsDev = await loadModelsDev(modelsDevSpec);
  } catch {}
  // #4544: never write sync-models-owned catalogs into the tracked default;
  // sync-models lands them in the runtime overlay instead.
  const applyReport = { ...report, providers: {} };
  for (const [pid, entry] of Object.entries(report.providers)) {
    if (SYNC_OWNED_PROVIDERS.has(pid) && (entry.additions_validated ?? []).length > 0) {
      console.log(
        `[catalog-sync] skipping ${pid}: ${entry.additions_validated.length} validated addition(s) owned by sync-models (runtime overlay)`,
      );
      defineValue(applyReport.providers, pid, { ...entry, additions_validated: [] });
    } else {
      defineValue(applyReport.providers, pid, entry);
    }
  }
  const { text, added } = applyAdditions(configText, config, applyReport, modelsDev);
  assertCatalogOnly(configText, text, added);
  fs.writeFileSync(configPath, text);
  const body = renderPrBody(applyReport, added);
  console.log(body);
  if (openPr) {
    const repoDir = opt('--repo', path.dirname(path.dirname(configPath)));
    const date = new Date().toISOString().slice(0, 10);
    const bodyFile = path.join(fs.mkdtempSync(path.join('/tmp', 'catalog-sync-')), 'pr-body.md');
    fs.writeFileSync(bodyFile, body);
    const run = (cmd) => execFileSync(cmd[0], cmd.slice(1), { cwd: repoDir, stdio: 'inherit' });
    const gitCapture = (...a) => execFileSync('git', a, { cwd: repoDir, encoding: 'utf8' }).trim();
    const branch = catalogSyncBranch(date);
    let branchExists = false;
    try {
      gitCapture('rev-parse', '--verify', '--quiet', `refs/heads/${branch}`);
      branchExists = true;
    } catch {}
    let defaultBranch = 'main';
    try {
      defaultBranch = gitCapture('symbolic-ref', '--short', 'refs/remotes/origin/HEAD').replace(
        /^origin\//,
        '',
      );
    } catch {}
    const commands = openPrCommands({ branch, branchExists, date, bodyFile });
    run(commands[0]); // branch step: create or reuse chore/catalog-sync-<date>
    assertNotDefaultBranch(gitCapture('branch', '--show-current'), defaultBranch);
    for (const cmd of commands.slice(1)) run(cmd);
  }
  return 0;
}

// Entry-point check: compare realpaths, not the raw strings (#2235;
// same pattern as proxy-supervise.mjs / #2083). process.argv[1] keeps the
// path exactly as invoked (e.g. through a file or directory symlink)
// while import.meta.url is the resolved module URL, so a string compare
// silently never matches and main() never runs via a symlink.
const isMain = (() => {
  try {
    if (!process.argv[1]) return false;
    return fs.realpathSync(process.argv[1]) === fs.realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
})();
if (isMain) {
  main(process.argv.slice(2)).then(
    (code) => process.exit(code),
    (err) => {
      if (err instanceof UsageError) {
        console.error(`[catalog-sync] ${err.message}`);
      } else {
        console.error(`[catalog-sync] fatal: ${err?.stack ?? err}`);
      }
      process.exit(2);
    },
  );
}
