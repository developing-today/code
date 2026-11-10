#!/usr/bin/env node

// Static route audit for the fleet's four operating tiers. It deliberately
// checks the tracked config only: built-in and runtime-overlay catalogs are
// unknown here, not evidence that a route is broken. Runtime-disabled leaves
// remain legal dormant seats, but are excluded from effective membership and
// can never prove live tier coverage.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseJsonc } from './provider-catalog-sync.mjs';
import { isMeteredModel } from './check-opencode-metered-providers.mjs';

export const DEFAULT_CONFIG = '.opencode/opencode.jsonc';
export const REQUIRED_TIERS = ['scout', 'workhorse', 'specialist', 'summit'];
// Keep aligned with bin/opencode-route-proxy.mjs BUILTIN_PROVIDERS. These
// concrete provider ids take precedence over a same-named provider route.
const BUILTIN_PROVIDER_IDS = new Set(['opencode-go', 'openai', 'deepseek', 'groq', 'openrouter', 'meta']);

function isObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function activeMap(config, key) {
  const experimental = config.experimental;
  const preferred = experimental?.[key];
  if (isObject(preferred) || Array.isArray(preferred)) return preferred;
  return config[key];
}

function normalizeWindowGate(window) {
  if (typeof window === 'string' && window) return { name: window, invert: false };
  if (isObject(window) && typeof window.name === 'string' && window.name) {
    return { name: window.name, invert: window.invert === true };
  }
  return null;
}

function windowIsPermanentlyClosed(window) {
  if (!isObject(window)) return true;
  const parseTime = (value) => {
    const match = /^([0-9]{1,2}):([0-9]{2})$/.exec(typeof value === 'string' ? value.trim() : '');
    if (!match) return null;
    const hour = Number(match[1]);
    const minute = Number(match[2]);
    if (hour > 23 || minute > 59) return null;
    return hour * 60 + minute;
  };
  const start = parseTime(window.start);
  const end = parseTime(window.end);
  if (start === null || end === null || start === end) return true;

  try {
    new Intl.DateTimeFormat('en-US', { timeZone: window.timezone || 'UTC' }).format(new Date(0));
  } catch {
    return true;
  }

  if (Array.isArray(window.daysOfWeek) && window.daysOfWeek.length > 0) {
    const weekdays = new Set(['sun', 'mon', 'tue', 'wed', 'thu', 'fri', 'sat']);
    const days = window.daysOfWeek.map((day) => String(day).slice(0, 3).toLowerCase());
    if (!days.some((day) => weekdays.has(day))) return true;
  }
  return false;
}

function splitModelReference(reference) {
  if (typeof reference !== 'string') return null;
  const slash = reference.indexOf('/');
  if (slash < 1 || slash === reference.length - 1) return null;
  const provider = reference.slice(0, slash);
  const suffix = reference.slice(slash + 1);
  const hash = suffix.lastIndexOf('#');
  const model = hash < 0 ? suffix : suffix.slice(0, hash);
  const effort = hash < 0 ? null : suffix.slice(hash + 1);
  if (!model || (hash >= 0 && !effort)) return null;
  return { provider, model, effort };
}

function catalogInfo(config, provider, model) {
  const seen = new Set();
  let id = provider;
  while (typeof id === 'string' && id && !seen.has(id)) {
    seen.add(id);
    const block = config.provider?.[id];
    if (!isObject(block)) return { catalog: 'unknown', exists: null, model: null };
    // Match the route proxy's providerDef: an explicit model map wins; when
    // absent, follow instanceOf to the tracked base provider.
    if (isObject(block.models)) {
      if (!Object.hasOwn(block.models, model)) {
        return { catalog: 'tracked', exists: false, model: null };
      }
      return { catalog: 'tracked', exists: true, model: block.models[model] };
    }
    id = block.instanceOf;
  }
  return { catalog: 'unknown', exists: null, model: null };
}

function formatCandidate(candidate) {
  return `${candidate.provider}/${candidate.model}${candidate.effort ? `#${candidate.effort}` : ''}`;
}

export function inspectRouteRules(config) {
  const issues = [];
  const issueSet = new Set();
  const addIssue = (issue) => {
    if (!issueSet.has(issue)) {
      issueSet.add(issue);
      issues.push(issue);
    }
  };
  const disabled = new Set(Array.isArray(config.disabled_providers) ? config.disabled_providers : []);
  const modelRoutes = activeMap(config, 'model_routes');
  const providerRoutes = activeMap(config, 'provider_routes');
  const modelRouteWindows = activeMap(config, 'model_route_windows');
  const tiers = {};

  const resolve = (reference, effort, tier, trace, stack, seen) => {
    const parsed = splitModelReference(reference);
    if (!parsed) {
      addIssue(`${tier}: invalid model target ${JSON.stringify(reference)} at ${trace}`);
      return [];
    }
    const selectedEffort = parsed.effort ?? effort;

    if (parsed.provider === 'opencode-route') {
      const route = modelRoutes?.[parsed.model];
      if (!isObject(route)) {
        addIssue(`${tier}: referenced model route "${parsed.model}" is missing at ${trace}`);
        return [];
      }
      if (stack.includes(parsed.model)) {
        addIssue(`${tier}: model-route cycle ${[...stack, parsed.model].join(' -> ')}`);
        return [];
      }
      // Match runtime resolveRoute: one shared seen set per tier
      // resolution, so a repeated nested-route reference is expanded
      // once. Ancestry (stack) still reports true cycles above.
      if (seen.has(parsed.model)) return [];
      seen.add(parsed.model);
      return resolveTargets(route, selectedEffort, tier, `${trace} -> ${parsed.model}`, [...stack, parsed.model], seen);
    }

    // Configured concrete providers take precedence over a same-named group,
    // matching providerRouteDef in the route proxy.
    const concreteProvider = isObject(config.provider?.[parsed.provider]) || BUILTIN_PROVIDER_IDS.has(parsed.provider);
    const providerRoute = concreteProvider ? null : providerRoutes?.[parsed.provider];
    if (isObject(providerRoute)) {
      // The runtime disables a provider-route group before expanding any
      // member. Keep a disabled placeholder for the audit report, but do not
      // let its enabled-looking members prove tier coverage.
      if (disabled.has(parsed.provider)) {
        return [{
          provider: parsed.provider,
          model: parsed.model,
          effort: selectedEffort,
          disabled: true,
          disabled_route: true,
          funds: isMeteredModel(`${parsed.provider}/${parsed.model}`),
          catalog: 'unknown',
          exists: null,
          trace,
        }];
      }
      if (!Array.isArray(providerRoute.providers) || providerRoute.providers.length === 0) {
        addIssue(`${tier}: provider route "${parsed.provider}" has no providers at ${trace}`);
        return [];
      }
      return providerRoute.providers.flatMap((provider) => {
        if (typeof provider !== 'string' || !provider) {
          addIssue(`${tier}: provider route "${parsed.provider}" has an invalid provider at ${trace}`);
          return [];
        }
        const concreteProvider = isObject(config.provider?.[provider]) || BUILTIN_PROVIDER_IDS.has(provider);
        if (provider === 'opencode-route' || !concreteProvider) {
          addIssue(`${tier}: provider route "${parsed.provider}" has non-concrete member "${provider}" at ${trace}`);
          return [];
        }
        if (disabled.has(provider)) {
          return appendLeaf(provider, parsed.model, selectedEffort, tier, `${trace} -> ${parsed.provider}`);
        }
        const catalog = catalogInfo(config, provider, parsed.model);
        // The runtime skips a group member whose known catalog lacks this
        // model; heterogeneous provider routes are valid.
        if (catalog.catalog === 'tracked' && !catalog.exists) return [];
        return appendLeaf(provider, parsed.model, selectedEffort, tier, `${trace} -> ${parsed.provider}`);
      });
    }

    return appendLeaf(parsed.provider, parsed.model, selectedEffort, tier, trace);
  };

  const appendLeaf = (provider, model, effort, tier, trace) => {
    const catalog = catalogInfo(config, provider, model);
    const candidate = {
      provider,
      model,
      effort,
      disabled: disabled.has(provider),
      funds: isMeteredModel(`${provider}/${model}`),
      catalog: catalog.catalog,
      exists: catalog.exists,
      trace,
    };
    if (!candidate.disabled && catalog.catalog === 'tracked' && !catalog.exists) {
      addIssue(`${tier}: ${provider}/${model} is missing from its tracked provider catalog (via ${trace})`);
    }
    return [candidate];
  };

  const resolveTargets = (route, incomingEffort, tier, trace, stack, seen) => {
    if (!Array.isArray(route.targets)) {
      addIssue(`${tier}: route "${trace.split(' -> ').at(-1)}" has no targets list`);
      return [];
    }
    // An empty target list is a deliberate chain stage that contributes
    // nothing (the runtime expands it to zero candidates). Tier coverage
    // is judged on the whole chain, so an emptied stage is not an issue.
    if (route.targets.length === 0) return [];
    return route.targets.flatMap((target, index) => {
      const targetTrace = `${trace}.targets[${index}]`;
      const windowGate = isObject(target) ? normalizeWindowGate(target.window) : null;
      if (windowGate && (!isObject(modelRouteWindows) || !Object.hasOwn(modelRouteWindows, windowGate.name))) {
        addIssue(`${tier}: target at ${targetTrace} names unknown model-route window "${windowGate.name}"`);
        // An unknown non-inverted gate is permanently closed in the router.
        if (!windowGate.invert) return [];
      } else if (windowGate && windowIsPermanentlyClosed(modelRouteWindows[windowGate.name])) {
        addIssue(`${tier}: target at ${targetTrace} uses permanently closed model-route window "${windowGate.name}"`);
        if (!windowGate.invert) return [];
      }
      let effort = incomingEffort;
      if (isObject(target) && isObject(target.variants) && effort !== null) {
        if (Object.hasOwn(target.variants, effort)) {
          const remapped = target.variants[effort];
          if (typeof remapped !== 'string' || !remapped) {
            addIssue(`${tier}: invalid effort remap for ${JSON.stringify(effort)} at ${targetTrace}`);
            return [];
          }
          effort = remapped;
        }
      }
      const reference = typeof target === 'string' ? target : isObject(target) ? target.model : null;
      if (typeof reference !== 'string') {
        addIssue(`${tier}: target at ${targetTrace} must be a model string or object with a model string`);
        return [];
      }
      return resolve(reference, effort, tier, targetTrace, stack, seen);
    });
  };

  for (const tier of REQUIRED_TIERS) {
    if (!isObject(modelRoutes) || !isObject(modelRoutes[tier])) {
      addIssue(`${tier}: required tier route is missing`);
      tiers[tier] = { members: [], disabled_excluded: [], funds_excluded: [], coverage: [] };
      continue;
    }
    const candidates = resolve(`opencode-route/${tier}`, null, tier, tier, [], new Set());
    const disabledExcluded = candidates.filter((candidate) => candidate.disabled);
    const fundsExcluded = candidates.filter((candidate) => !candidate.disabled && candidate.funds);
    const members = candidates.filter((candidate) => !candidate.disabled);
    const eligible = members.filter((candidate) => !candidate.funds);
    const coverage = eligible.filter((candidate) => candidate.catalog === 'tracked' && candidate.exists);

    if (coverage.length === 0) {
      addIssue(`${tier}: no enabled, tracked, existing, non-funds model provides live coverage`);
    }

    const pinned = eligible.filter((candidate) => typeof candidate.effort === 'string' && candidate.effort.length > 0);
    if (pinned.length === 0) {
      addIssue(`${tier}: no enabled non-funds model has an explicit effort variant`);
    }

    let effortVariantsChecked = 0;
    for (const candidate of eligible) {
      if (candidate.catalog !== 'tracked' || !candidate.exists) continue;
      const modelInfo = catalogInfo(config, candidate.provider, candidate.model).model;
      if (!isObject(modelInfo?.variants)) continue;
      effortVariantsChecked += 1;
      if (!candidate.effort) {
        addIssue(`${tier}: ${candidate.provider}/${candidate.model} has no effort selected although its tracked catalog defines variants`);
      } else if (!Object.hasOwn(modelInfo.variants, candidate.effort)) {
        addIssue(`${tier}: ${formatCandidate(candidate)} is not a declared variant in its tracked catalog`);
      }
    }

    tiers[tier] = {
      candidate_count: candidates.length,
      members,
      disabled_excluded: disabledExcluded,
      funds_excluded: fundsExcluded,
      coverage,
      effort_pinned: pinned.length,
      effort_variants_checked: effortVariantsChecked,
      unknown_catalogs: eligible.filter((candidate) => candidate.catalog === 'unknown').length,
    };
  }

  return { tiers, issues };
}

export function checkRouteRules(source) {
  return inspectRouteRules(typeof source === 'string' ? parseJsonc(source) : source).issues;
}

export function parseArgs(argv) {
  let configPath = null;
  let json = false;
  for (const token of argv) {
    if (token === '--json') {
      if (json) throw new Error('--json was given more than once');
      json = true;
    } else if (token.startsWith('--')) {
      throw new Error(`unrecognised argument: ${token}`);
    } else if (configPath) {
      throw new Error(`unexpected positional argument: ${token}`);
    } else {
      configPath = token;
    }
  }
  return { configPath: path.resolve(configPath ?? DEFAULT_CONFIG), json };
}

async function main(argv) {
  const { configPath, json } = parseArgs(argv);
  const result = inspectRouteRules(parseJsonc(fs.readFileSync(configPath, 'utf8')));
  if (json) {
    console.log(JSON.stringify({ config: configPath, ...result }, null, 2));
  } else {
    for (const tier of REQUIRED_TIERS) {
      const summary = result.tiers[tier];
      console.log(
        `${tier}: ${summary.coverage.length} tracked live coverage model(s); ${summary.members.length} active non-disabled candidate(s); ${summary.disabled_excluded.length} disabled seat(s) excluded; ${summary.effort_pinned} effort-pinned candidate(s), ${summary.effort_variants_checked} checked against tracked variants, ${summary.unknown_catalogs} catalog-unknown`,
      );
    }
    for (const issue of result.issues) console.error(`- ${issue}`);
    if (result.issues.length === 0) console.log(`Route-rule guard passed: ${configPath}`);
  }
  if (result.issues.length > 0) process.exitCode = 1;
}

const isMain = (() => {
  try {
    return process.argv[1] && fs.realpathSync(process.argv[1]) === fs.realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
})();

if (isMain) {
  try {
    await main(process.argv.slice(2));
  } catch (error) {
    console.error(`[route-rules] ${error?.message ?? error}`);
    process.exitCode = 2;
  }
}
