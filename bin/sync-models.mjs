#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { execSync } from 'node:child_process';

const DRY_RUN = process.argv.includes('--dry-run');
const VERBOSE = process.argv.includes('--verbose') || process.argv.includes('-v');

const ROOT_DIR = path.resolve(import.meta.dirname, '..');
const OPENCODE_CONFIG_PATH = path.join(ROOT_DIR, '.opencode/opencode.jsonc');
const HOME_DIR = process.env.HOME || '/home/user';
const CODEX_CACHE_PATH = path.join(HOME_DIR, '.codex/models_cache.json');
const GEMINI_CACHE_PATH = path.join(HOME_DIR, '.gemini/antigravity_models_cache.json');

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

// -----------------------------------------------------------------------------
// 2. Discover Antigravity Models (via Language Server RPC or agy CLI)
// -----------------------------------------------------------------------------
async function discoverAntigravityModels() {
  console.log('[sync-models] Discovering Antigravity models...');
  const discovered = [];

  // Try 1: Language Server RPC GetAvailableModels
  try {
    let port = 46713;
    let token = process.env.ANTIGRAVITY_CSRF_TOKEN || null;

    if (!token) {
      try {
        const ps = execSync('ps aux | grep language_server | grep -v grep', { encoding: 'utf-8' });
        const m = ps.match(/--csrf_token\s+([a-zA-Z0-9_-]+)/);
        if (m) token = m[1];
      } catch {}
    }

    if (token) {
      const res = await fetch(`http://127.0.0.1:${port}/exa.language_server_pb.LanguageServerService/GetAvailableModels`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'x-codeium-csrf-token': token
        },
        body: '{}',
        signal: AbortSignal.timeout(3000)
      });
      if (res.ok) {
        const json = await res.json();
        const models = json.response?.models || {};
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
    }
  } catch {}

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
// Helper: Clean JSONC parser & formatter
// -----------------------------------------------------------------------------
function parseJsonc(text) {
  const cleanLines = text.split('\n').filter(l => {
    const s = l.trim();
    return !s.startsWith('//') && !s.startsWith('/*');
  }).map(l => l.replace(/\s+\/\/.*$/, ''));
  const cleaned = cleanLines.join('\n').replace(/,\s*([}\]])/g, '$1');
  return JSON.parse(cleaned);
}

// -----------------------------------------------------------------------------
// Main Sync Runner
// -----------------------------------------------------------------------------
async function run() {
  const modelsDev = await fetchModelsDev();
  const agyModels = await discoverAntigravityModels();
  const codexModels = discoverCodexModels();

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

  // 1. Sync antigravity-agy & antigravity-ls
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

    for (const targetProvider of ['antigravity-agy', 'antigravity-ls']) {
      if (!config.provider[targetProvider]) continue;
      const prov = config.provider[targetProvider];
      prov.models = prov.models || {};

      for (const [baseId, info] of Object.entries(grouped)) {
        const limits = defaultLimits(baseId);
        const variants = {};
        for (const eff of Array.from(info.efforts).sort()) {
          variants[eff] = { options: { reasoning_effort: eff } };
        }

        const modelEntry = {
          name: `${info.name} (${targetProvider === 'antigravity-ls' ? 'Direct LS' : 'agy'})`,
          limit: limits
        };
        if (Object.keys(variants).length > 0) {
          modelEntry.variants = variants;
        }

        if (!prov.models[baseId]) {
          console.log(`[sync-models] + Adding ${targetProvider}/${baseId}`);
          prov.models[baseId] = modelEntry;
          totalAdded++;
        } else {
          let changed = false;
          if (Object.keys(variants).length > 0 && !prov.models[baseId].variants) {
            prov.models[baseId].variants = variants;
            changed = true;
          }
          if (prov.models[baseId].limit?.context !== limits.context) {
            prov.models[baseId].limit = limits;
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
        totalAdded++;
      } else {
        if (codexProv.models[m.slug].limit?.context !== 1000000) {
          codexProv.models[m.slug].limit = { context: 1000000, output: 128000 };
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
          totalAdded++;
        }
      }
    }
  }

  console.log(`[sync-models] Summary: ${totalAdded} models added, ${totalUpdated} models updated.`);

  if (totalAdded > 0 || totalUpdated > 0) {
    if (DRY_RUN) {
      console.log('[sync-models] Dry run complete. No files written.');
    } else {
      const backupPath = `${OPENCODE_CONFIG_PATH}.backup`;
      fs.writeFileSync(backupPath, origConfigText);
      console.log(`[sync-models] Created backup at ${backupPath}`);

      fs.writeFileSync(OPENCODE_CONFIG_PATH, JSON.stringify(config, null, 2) + '\n');
      console.log(`[sync-models] Successfully updated ${OPENCODE_CONFIG_PATH}!`);

      try {
        execSync('opencode reload', { stdio: 'ignore', timeout: 5000 });
        console.log('[sync-models] Triggered opencode configuration reload.');
      } catch {
        // Ignored if OpenCode background service is not running
      }
    }
  } else {
    console.log('[sync-models] All providers are already up-to-date!');
  }
}

run().catch(err => {
  console.error('[sync-models] Fatal error:', err);
  process.exit(1);
});
