#!/usr/bin/env node
// free-context-probe.mjs — bounded free-model context/route measurement harness (issue #9573, task 23761).
//
// Operator script argument contract (#3352): this file declares its whole
// command line in VALUE_OPTIONS / FLAG_OPTIONS and parses strictly via
// parseArgs() before any file read, network call, or file write. Unknown
// flags, stray positionals, typo'd flags, missing values, and duplicate
// value options are UsageError (exit 2) naming the offending token.
// --opt=value works identically to --opt value; a present-but-valueless
// option is a usage error, never the default.
//
// Modes:
//   --inventory-only   read .opencode/opencode.jsonc, print the exact wired
//                      free leaf inventory as JSON (no network).
//   (default dry-run)  build one deterministic payload shape per leaf and
//                      write a manifest with cost controls (no network).
//   --live             single-leaf bounded live probe (requires --leaf,
//                      refuses multi-leaf sweeps; one attempt, strict
//                      timeout, sanitized metadata only, no secrets logged).
//                      Keyless for kilo/kilo-auto/free; credentialed direct
//                      adapters (groq/nvidia/openrouter, slice 4) fail
//                      closed before any network call when unsupported.
//
// Safety: recommendation-only, no routing configuration changes. Live
// probes resolve credentials on the authorized host from the environment
// only and never print headers, keys, or raw error bodies.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseJsonc } from './provider-catalog-sync.mjs';

// ---------------------------------------------------------------------------
// Strict argv parsing (#3352)
// ---------------------------------------------------------------------------
export const VALUE_OPTIONS = ['--config', '--out', '--max-probes', '--timeout-ms', '--max-input-tokens', '--plan', '--leaf'];
export const FLAG_OPTIONS = ['--live', '--dry-run', '--json', '--help', '--inventory-only'];
export const USAGE =
  'usage: free-context-probe [--config <path>] [--out <path>] [--max-probes <n>] ' +
  '[--timeout-ms <ms>] [--max-input-tokens <n>] [--plan <plan-id>] [--leaf <provider/model>] ' +
  '[--live] [--dry-run] [--json] [--inventory-only]';

export class UsageError extends Error {}

export function parseArgs(argv) {
  const options = {};
  const flags = new Set();
  for (let i = 0; i < argv.length; i += 1) {
    let token = argv[i];
    let inlineValue = null;
    const eq = token.indexOf('=');
    if (eq > 0 && token.startsWith('--')) {
      inlineValue = token.slice(eq + 1);
      token = token.slice(0, eq);
    }
    if (FLAG_OPTIONS.includes(token)) {
      if (inlineValue !== null) throw new UsageError(`${token} takes no value\n${USAGE}`);
      if (flags.has(token)) throw new UsageError(`${token} was given more than once\n${USAGE}`);
      flags.add(token);
      continue;
    }
    if (VALUE_OPTIONS.includes(token)) {
      if (Object.hasOwn(options, token)) throw new UsageError(`${token} was given more than once\n${USAGE}`);
      let value = inlineValue;
      if (value === null) {
        value = argv[i + 1];
        if (value === undefined || value.startsWith('--')) {
          throw new UsageError(
            `${token} requires a value, but ${value === undefined ? 'no value followed it' : `the next token is the flag ${value}`}\n${USAGE}`,
          );
        }
        i += 1;
      } else if (value === '') {
        throw new UsageError(`${token} requires a value, but an empty --opt= value was given\n${USAGE}`);
      }
      options[token] = value;
      continue;
    }
    throw new UsageError(`unrecognised argument: ${token}\n${USAGE}`);
  }
  return { options, flags };
}

// ---------------------------------------------------------------------------
// Free leaf inventory: exact wired leaves, derived from the config file.
// Curated plan list read 2026-11-25; extractFreeLeaves() fails loudly if a
// listed plan vanishes so drift is visible instead of silent.
// ---------------------------------------------------------------------------
export const FREE_PLAN_IDS = [
  'flash-free-plan',
  'groq-free-plan',
  'nvidia-free-plan',
  'openrouter-free-plan',
  'commandcode-free-plan',
  'kilo-free-plan',
  'glm-flash-free-plan',
  'gemini-free-plan',
  'exaone-free-plan',
  'beam-plan',
  'longcat-plan',
  'iceman-plan',
  'hy4-flash-plan',
  'inkling-plan',
  'glm-5-3-flash-zen-plan',
  'flint-plan',
  'big-pickle-plan',
  'nemotron-vl-plan',
  'nemotron-lightning-plan',
  'mimo-flash-plan',
];

export function splitTarget(target) {
  const ref = typeof target === 'string' ? target : target?.model;
  if (typeof ref !== 'string') return null;
  const hash = ref.indexOf('#');
  const bare = hash >= 0 ? ref.slice(0, hash) : ref;
  const variant = hash >= 0 ? ref.slice(hash + 1) : null;
  const slash = bare.indexOf('/');
  if (slash < 0) return null;
  return { target: ref, provider: bare.slice(0, slash), model: bare.slice(slash + 1), variant };
}

export function extractFreeLeaves(config) {
  const modelRoutes = config?.experimental?.model_routes;
  if (!modelRoutes || typeof modelRoutes !== 'object') throw new Error('extractFreeLeaves: no experimental.model_routes in config');
  const providerRoutes = config?.experimental?.provider_routes ?? {};
  const leaves = [];
  const missingPlans = [];
  for (const plan of FREE_PLAN_IDS) {
    const route = modelRoutes[plan];
    if (!route) {
      missingPlans.push(plan);
      continue;
    }
    const targets = route.targets ?? [];
    for (const t of targets) {
      const split = splitTarget(t);
      if (!split) continue;
      // Skip route-to-route references (opencode-route/*): only direct leaves count.
      if (split.provider === 'opencode-route') continue;
      const routedProviders = providerRoutes[split.provider]?.providers;
      const concreteProviders = Array.isArray(routedProviders) ? routedProviders : [split.provider];
      for (const provider of concreteProviders) {
        leaves.push({
          plan,
          ...split,
          provider,
          modelReference: split.target,
          providerRoute: Array.isArray(routedProviders) ? split.provider : null,
          contextSource: 'provider-published',
        });
      }
    }
  }
  return { leaves, missingPlans };
}

// ---------------------------------------------------------------------------
// Deterministic payload family: fixed blocks plus end sentinel. A pass is
// the server accepting the request; a failure is an explicit context-length
// rejection or sentinel truncation. Binary-search the input size between
// the highest repeatable pass and first repeatable fail.
// ---------------------------------------------------------------------------
export const PROBE_SENTINEL = 'SENTINEL_CTXPROBE_END_9f3c';
export const PROBE_BLOCK = 'CTXPROBE block 0123456789 ABCDEF abcdef mediana inquiry packet delta\n';

export function buildProbePayload({ blocks }) {
  if (!Number.isInteger(blocks) || blocks < 1) throw new Error('buildProbePayload: blocks must be a positive integer');
  const body = PROBE_BLOCK.repeat(blocks);
  return `${body}${PROBE_SENTINEL}\n`;
}

export function approxInputTokens(text) {
  return Math.ceil(text.length / 4);
}

// ---------------------------------------------------------------------------
// Result classification (oracle: mapping table below, tested offline).
// ---------------------------------------------------------------------------
export function classifyResult({ httpStatus, bodyText }) {
  const body = typeof bodyText === 'string' ? bodyText.toLowerCase() : '';
  if (httpStatus === 200) return { outcome: 'pass', reason: 'http-200' };
  const contextSignals = ['context_length_exceeded', 'maximum context length', 'exceeds the model context window', 'input tokens exceed the limit'];
  if (contextSignals.some((s) => body.includes(s))) return { outcome: 'fail-context', reason: 'context-signal' };
  if (httpStatus === 429) return { outcome: 'fail-rate', reason: 'http-429' };
  if (httpStatus === 401 || httpStatus === 402 || httpStatus === 403) return { outcome: 'fail-auth', reason: `http-${httpStatus}` };
  if (httpStatus === 404 || httpStatus === 410) return { outcome: 'fail-gone', reason: `http-${httpStatus}` };
  if (httpStatus === 422 || httpStatus === 500) return { outcome: 'fail-shape', reason: `http-${httpStatus}` };
  if (typeof httpStatus === 'number') return { outcome: 'fail-other', reason: `http-${httpStatus}` };
  return { outcome: 'fail-transient', reason: 'no-response' };
}

export function tagContextSource({ empiricallyTested }) {
  return empiricallyTested === true ? 'empirically-tested' : 'provider-published';
}

// Bound response memory and retain the timeout while reading the body.
export async function readBoundedBody(response, { maxBytes = 65536 } = {}) {
  const reader = response.body.getReader();
  const chunks = [];
  let length = 0;
  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    length += value.length;
    if (length > maxBytes) {
      await reader.cancel();
      throw new Error('response-too-large');
    }
    chunks.push(value);
  }
  return Buffer.concat(chunks).toString('utf8');
}

// Deliberately no OpenCode process: agent prompts, tools and fallback routes
// would invalidate a one-request budget. This first live slice is keyless
// Kilo only, at a fixed endpoint with an explicit free model and no paid tail.
export async function runKiloProbe({ prompt, timeoutMs, fetchImpl = fetch }) {
  const response = await fetchImpl('https://ai-gateway.kilo.ai/api/v1/chat/completions', {
    method: 'POST',
    redirect: 'error',
    headers: { 'content-type': 'application/json' },
    signal: AbortSignal.timeout(timeoutMs),
    body: JSON.stringify({
      model: 'kilo-auto/free',
      messages: [{ role: 'user', content: prompt }],
      max_tokens: 64,
      stream: false,
    }),
  });
  const text = await readBoundedBody(response);
  let data;
  try { data = JSON.parse(text); } catch { data = {}; }
  const usage = data.usage;
  return {
    ...classifyResult({ httpStatus: response.status, bodyText: text }),
    httpStatus: response.status,
    responseModel: typeof data.model === 'string' ? data.model : null,
    reportedUpstream: typeof data.provider === 'string' ? data.provider : null,
    usage: {
      input: Number.isFinite(usage?.prompt_tokens) ? usage.prompt_tokens : null,
      output: Number.isFinite(usage?.completion_tokens) ? usage.completion_tokens : null,
    },
    sentinelReturned: data.choices?.[0]?.message?.content?.trim() === PROBE_SENTINEL,
    finishReason: data.choices?.[0]?.finish_reason ?? null,
  };
}

// ---------------------------------------------------------------------------
// Credentialed direct adapters (slice 4, task 26036): one bounded live call
// to an exact non-Kilo leaf over the provider's public OpenAI-compatible
// endpoint. Every check fails closed BEFORE any network call: unsupported
// provider, exact leaf absent from the wired inventory, output cap above
// the hard cap, named credential env var absent, or run call budget
// exhausted. Credentials are read from the environment by name only and
// are never returned, logged, or written to a record (records carry the
// env var NAME as credentialSource). Recommendation-only: these adapters
// measure direct provider APIs; they change no routing configuration.
// ---------------------------------------------------------------------------
export const DIRECT_PROBE_MAX_CALLS = 5;
export const DIRECT_PROBE_MAX_OUTPUT_TOKENS = 64;

export class ProbeError extends Error {
  constructor(code, message) {
    super(message);
    this.code = code;
  }
}

export const DIRECT_ADAPTERS = {
  groq: {
    provider: 'groq',
    endpoint: 'https://api.groq.com/openai/v1/chat/completions',
    credentialEnv: 'GROQ_API_KEY',
    source: 'Groq OpenAI-compatibility docs (console.groq.com/docs): OpenAI-compatible chat completions; Free Plan key via GROQ_API_KEY',
  },
  nvidia: {
    provider: 'nvidia',
    endpoint: 'https://integrate.api.nvidia.com/v1/chat/completions',
    credentialEnv: 'NVIDIA_API_KEY',
    source: 'NVIDIA API Catalog getting started (build.nvidia.com): base https://integrate.api.nvidia.com/v1, Authorization Bearer $NVIDIA_API_KEY, free trial tier',
  },
  openrouter: {
    provider: 'openrouter',
    endpoint: 'https://openrouter.ai/api/v1/chat/completions',
    credentialEnv: 'OPENROUTER_API_KEY',
    source: 'OpenRouter docs (openrouter.ai/docs): OpenAI-compatible chat completions, Authorization Bearer key via OPENROUTER_API_KEY',
  },
};

export function createProbeBudget({ maxCalls = 1 } = {}) {
  if (!Number.isInteger(maxCalls) || maxCalls < 1 || maxCalls > DIRECT_PROBE_MAX_CALLS) {
    throw new ProbeError(
      'call-budget-violation',
      `probe budget maxCalls must be an integer in [1, ${DIRECT_PROBE_MAX_CALLS}], got ${maxCalls}`,
    );
  }
  let used = 0;
  return {
    maxCalls,
    get used() {
      return used;
    },
    consume() {
      if (used >= maxCalls) {
        throw new ProbeError('call-budget-exceeded', `probe call budget exhausted (${used}/${maxCalls}); no further live calls in this run`);
      }
      used += 1;
    },
  };
}

export function resolveDirectAdapter({ leaf, leaves, env = process.env, maxOutputTokens = DIRECT_PROBE_MAX_OUTPUT_TOKENS }) {
  const adapter = leaf ? DIRECT_ADAPTERS[leaf.provider] : undefined;
  if (!adapter) {
    throw new ProbeError(
      'unsupported-provider',
      `no direct adapter for provider ${leaf?.provider ?? '(none)'}; supported: ${Object.keys(DIRECT_ADAPTERS).join(', ')} (kilo stays on its keyless path)`,
    );
  }
  const inInventory = Array.isArray(leaves) && leaves.some((l) => l.provider === leaf.provider && l.model === leaf.model);
  if (!inInventory) {
    throw new ProbeError(
      'unsupported-model',
      `exact leaf ${leaf.provider}/${leaf.model} is not in the wired free inventory; refusing to probe an unwired model`,
    );
  }
  if (!Number.isInteger(maxOutputTokens) || maxOutputTokens < 1 || maxOutputTokens > DIRECT_PROBE_MAX_OUTPUT_TOKENS) {
    throw new ProbeError(
      'output-cap-violation',
      `maxOutputTokens must be an integer in [1, ${DIRECT_PROBE_MAX_OUTPUT_TOKENS}], got ${maxOutputTokens}`,
    );
  }
  const credential = env?.[adapter.credentialEnv];
  if (typeof credential !== 'string' || credential.length === 0) {
    throw new ProbeError(
      'missing-credentials',
      `credential env ${adapter.credentialEnv} is not set for provider ${adapter.provider}; failing closed with no network call`,
    );
  }
  return { adapter, credentialSource: `env:${adapter.credentialEnv}` };
}

export async function runDirectProbe({
  leaf,
  leaves,
  prompt,
  timeoutMs,
  maxOutputTokens = DIRECT_PROBE_MAX_OUTPUT_TOKENS,
  env = process.env,
  fetchImpl = fetch,
  budget,
}) {
  const { adapter, credentialSource } = resolveDirectAdapter({ leaf, leaves, env, maxOutputTokens });
  if (!budget || typeof budget.consume !== 'function') {
    throw new ProbeError('call-budget-violation', 'runDirectProbe requires an explicit probe budget from createProbeBudget()');
  }
  budget.consume();
  const response = await fetchImpl(adapter.endpoint, {
    method: 'POST',
    redirect: 'error',
    headers: { 'content-type': 'application/json', authorization: `Bearer ${env[adapter.credentialEnv]}` },
    signal: AbortSignal.timeout(timeoutMs),
    body: JSON.stringify({
      model: leaf.model,
      messages: [{ role: 'user', content: prompt }],
      max_tokens: maxOutputTokens,
      stream: false,
    }),
  });
  const text = await readBoundedBody(response);
  let data;
  try {
    data = JSON.parse(text);
  } catch {
    data = {};
  }
  const usage = data.usage;
  const outputTokens = Number.isFinite(usage?.completion_tokens) ? usage.completion_tokens : null;
  const classified = classifyResult({ httpStatus: response.status, bodyText: text });
  let { outcome, reason } = classified;
  // Fail closed on a provider that reports output beyond the requested cap:
  // the measurement is invalid, never a pass.
  if (outcome === 'pass' && outputTokens !== null && outputTokens > maxOutputTokens) {
    outcome = 'fail-cap';
    reason = 'output-cap-violation';
  }
  const responseModel = typeof data.model === 'string' ? data.model : null;
  return {
    outcome,
    reason,
    httpStatus: response.status,
    adapter: adapter.provider,
    credentialSource,
    responseModel,
    reportedUpstream: typeof data.provider === 'string' ? data.provider : null,
    usage: {
      input: Number.isFinite(usage?.prompt_tokens) ? usage.prompt_tokens : null,
      output: outputTokens,
    },
    sentinelReturned: data.choices?.[0]?.message?.content?.trim() === PROBE_SENTINEL,
    finishReason: data.choices?.[0]?.finish_reason ?? null,
    servingProvider: adapter.provider,
    servingLeaf: responseModel ? `${adapter.provider}/${responseModel}` : null,
  };
}

function resolveConfigPath({ options, root }) {
  return options['--config'] ?? path.join(root, '.opencode', 'opencode.jsonc');
}

async function main(argv) {
  const { options, flags } = parseArgs(argv);
  if (flags.has('--help')) {
    console.log(USAGE);
    return 0;
  }
  if (flags.has('--live') && flags.has('--dry-run')) {
    throw new UsageError(`--live and --dry-run are mutually exclusive\n${USAGE}`);
  }
  if (flags.has('--live') && flags.has('--inventory-only')) {
    throw new UsageError(`--live and --inventory-only are mutually exclusive\n${USAGE}`);
  }
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
  const configPath = resolveConfigPath({ options, root });
  const configText = fs.readFileSync(configPath, 'utf8');
  const config = parseJsonc(configText);
  const { leaves, missingPlans } = extractFreeLeaves(config);

  let filtered = leaves;
  if (options['--plan']) filtered = filtered.filter((l) => l.plan === options['--plan']);
  if (options['--leaf']) {
    const want = options['--leaf'];
    filtered = filtered.filter((l) => `${l.provider}/${l.model}` === want || l.target === want || l.modelReference === want);
  }

  if (flags.has('--inventory-only')) {
    const out = { generatedAt: new Date().toISOString(), leafCount: filtered.length, missingPlans, leaves: filtered };
    if (options['--out']) fs.writeFileSync(options['--out'], `${JSON.stringify(out, null, 2)}\n`);
    else console.log(JSON.stringify(out, null, 2));
    if (missingPlans.length > 0) {
      console.error(`[free-context-probe] warning: ${missingPlans.length} listed plan(s) absent from config: ${missingPlans.join(', ')}`);
      return 3;
    }
    return 0;
  }

  const maxProbes = Number(options['--max-probes'] ?? '41');
  const timeoutMs = Number(options['--timeout-ms'] ?? '30000');
  const maxInputTokens = Number(options['--max-input-tokens'] ?? '8000');
  if (!Number.isInteger(maxProbes) || maxProbes < 1) throw new UsageError(`--max-probes must be a positive integer\n${USAGE}`);
  if (!Number.isInteger(timeoutMs) || timeoutMs < 1000 || timeoutMs > 120000) {
    throw new UsageError(`--timeout-ms must be an integer in [1000, 120000]\n${USAGE}`);
  }
  if (!Number.isInteger(maxInputTokens) || maxInputTokens < 256) throw new UsageError(`--max-input-tokens must be an integer >= 256\n${USAGE}`);

  if (flags.has('--live')) {
    // Cost control: live mode is exactly one call to one exact leaf — the
    // keyless Kilo adapter, or one credentialed direct adapter (slice 4)
    // that fails closed before any network call when its provider, model,
    // credential, output cap, or call budget check fails.
    if (!options['--leaf']) throw new UsageError(`--live requires an exact --leaf\n${USAGE}`);
    if (filtered.length !== 1) throw new UsageError(`--live matched ${filtered.length} leaves; give an exact --leaf\n${USAGE}`);
    if (options['--max-probes'] !== undefined && Number(options['--max-probes']) !== 1) {
      throw new UsageError(`--live requires --max-probes 1\n${USAGE}`);
    }
    const leaf = filtered[0];
    // This adapter only permits a bounded 8-block payload (about 160 chars),
    // while the configured budget applies to payload + instructions. The
    // chars/4 figure is a rough preflight estimate, not a tokenizer oracle.
    const payload = buildProbePayload({ blocks: 8 });
    const prompt = `Repeat the final sentinel exactly and nothing else. Input payload follows:\n${payload}`;
    const estimatedPromptTokens = approxInputTokens(prompt);
    if (estimatedPromptTokens > maxInputTokens) {
      throw new UsageError(`live probe payload exceeds --max-input-tokens ${maxInputTokens}\n${USAGE}`);
    }
    const started = Date.now();
    const isKilo = leaf.provider === 'kilo';
    let liveResult;
    if (isKilo) {
      try {
        liveResult = await runKiloProbe({ prompt, timeoutMs });
      } catch {
        liveResult = { outcome: 'fail-transient', reason: 'request-error', httpStatus: null, responseModel: null, reportedUpstream: null, usage: { input: null, output: null }, sentinelReturned: false };
      }
    } else {
      try {
        liveResult = await runDirectProbe({
          leaf,
          leaves,
          prompt,
          timeoutMs,
          budget: createProbeBudget({ maxCalls: 1 }),
        });
      } catch (err) {
        if (err instanceof ProbeError) {
          // Fail closed: no record is written for a probe that never ran.
          console.error(`[free-context-probe] live direct probe failed closed (${err.code}): ${err.message}`);
          return 4;
        }
        liveResult = { outcome: 'fail-transient', reason: 'request-error', httpStatus: null, responseModel: null, reportedUpstream: null, usage: { input: null, output: null }, sentinelReturned: false };
      }
    }
    const pass = liveResult.outcome === 'pass' && liveResult.sentinelReturned;
    const record = {
      at: new Date().toISOString(),
      mode: 'live',
      requestedLeaf: `${leaf.provider}/${leaf.model}`,
      plan: leaf.plan,
      inputChars: payload.length,
      approxInputTokens: estimatedPromptTokens,
      inputTokenEstimate: 'rough chars/4; not a provider tokenizer; live adapter also bounds this payload to 8 fixed blocks',
      timeoutMs,
      maxInputTokens,
      maxOutputTokens: 64,
      attempts: 1,
      latencyMs: Date.now() - started,
      outcome: pass ? 'pass-sentinel' : liveResult.outcome,
      reason: pass ? 'sentinel-returned' : liveResult.reason,
      httpStatus: liveResult.httpStatus,
      responseModel: liveResult.responseModel,
      reportedUpstream: liveResult.reportedUpstream,
      servingProvider: isKilo ? 'kilo' : liveResult.servingProvider,
      servingLeaf: liveResult.servingLeaf ?? (liveResult.responseModel ? `${leaf.provider}/${liveResult.responseModel}` : null),
      adapter: isKilo ? 'kilo-keyless' : liveResult.adapter,
      credentialSource: isKilo ? 'none-keyless' : liveResult.credentialSource,
      sentinelReturned: liveResult.sentinelReturned,
      usage: liveResult.usage,
      contextSource: 'provider-published',
      note: 'Single small sentinel smoke probe only; request has a 64-token output cap. Timeout is not proof of zero provider cost. This is not a context-ceiling test.',
    };
    if (options['--out']) fs.writeFileSync(options['--out'], `${JSON.stringify(record, null, 2)}\n`);
    else console.log(JSON.stringify(record, null, 2));
    return liveResult.httpStatus === 200 ? 0 : 4;
  }

  // Default: dry-run manifest (no network). One deterministic shape per leaf.
  const probeBlocks = Math.max(1, Math.floor(maxInputTokens / approxInputTokens(PROBE_BLOCK.repeat(8)) / 8) || 1);
  const manifest = {
    generatedAt: new Date().toISOString(),
    mode: 'dry-run',
    costControls: { maxProbes, timeoutMs, maxInputTokens, attemptsPerLeaf: 1, networkCalls: 0 },
    leafCount: Math.min(filtered.length, maxProbes),
    missingPlans,
    probes: filtered.slice(0, maxProbes).map((leaf) => {
      const payload = buildProbePayload({ blocks: Math.min(probeBlocks, 8) });
      return {
        requestedLeaf: `${leaf.provider}/${leaf.model}`,
        plan: leaf.plan,
        target: leaf.target,
        inputChars: payload.length,
        approxInputTokens: approxInputTokens(payload),
        sentinel: PROBE_SENTINEL,
        contextSource: tagContextSource({ empiricallyTested: false }),
      };
    }),
  };
  if (options['--out']) fs.writeFileSync(options['--out'], `${JSON.stringify(manifest, null, 2)}\n`);
  else console.log(JSON.stringify(manifest, null, 2));
  return missingPlans.length > 0 ? 3 : 0;
}

const invokedAsScript = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedAsScript) {
  main(process.argv.slice(2)).then(
    (code) => process.exit(code ?? 0),
    (err) => {
      if (err instanceof UsageError) {
        console.error(`[free-context-probe] ${err.message}`);
        process.exit(2);
      }
      console.error(`[free-context-probe] ${err?.stack ?? err}`);
      process.exit(1);
    },
  );
}
