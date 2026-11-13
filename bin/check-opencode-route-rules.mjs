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

// Class-level fallback layout (Drew 2026-11-13; full table in
// .opencode/route-fallbacks.md). Semantics: a class route's own regular
// chain always runs first and completely; the fallback targets appended
// after it are a second pass that engages only on exhaustion, walked
// transitively under a per-call visited set (a class already tried in
// the call is never re-entered), terminating at specialist, the sink.
// The pairs workhorse->scout / scout->workhorse are therefore legal
// fallback edges, NOT cycles: the runtime route expander prunes
// re-entry into a route already on the expansion path. scout's and
// background's edges are proposals Drew has not ruled on yet
// (2026-11-13); the rest are his confirmed edges. Keep this table and
// the config in lockstep - check (a) below fails on drift.
export const CLASS_FALLBACKS = {
  small: ['speed'],
  speed: ['workhorse', 'summit', 'specialist'],
  workhorse: ['scout', 'specialist'],
  summit: ['specialist'],
  specialist: [],
  scout: ['workhorse'],
  background: ['workhorse'],
};
const CLASS_FALLBACK_PAIRS = new Set(
  Object.entries(CLASS_FALLBACKS).flatMap(([from, targets]) => targets.map((to) => `${from}->${to}`)),
);

function routeTargetIds(route, modelRoutes) {
  if (!isObject(route) || !Array.isArray(route.targets)) return [];
  const ids = [];
  for (const target of route.targets) {
    const reference = typeof target === 'string' ? target : isObject(target) ? target.model : null;
    if (typeof reference !== 'string') continue;
    const parsed = splitModelReference(reference);
    if (parsed && parsed.provider === 'opencode-route' && isObject(modelRoutes?.[parsed.model])) {
      ids.push(parsed.model);
    }
  }
  return ids;
}

function cycleKey(cyclePath) {
  return [...new Set(cyclePath)].sort().join(',');
}

// Validates the class fallback layout (checks a-c) and the static
// composition graph (check d) for a parsed model_routes map.
// fallbacks defaults to CLASS_FALLBACKS; tests pass doctored tables to
// prove the reachability/walk checks fire. reportedCycleKeys holds
// canonical keys of cycles the tier resolver already reported, so a
// composition cycle is never double-reported.
export function inspectFallbackLayout(modelRoutes, fallbacks = CLASS_FALLBACKS, reportedCycleKeys = new Set()) {
  const issues = [];
  if (!isObject(modelRoutes)) return issues;
  const fallbackPairs = new Set(
    Object.entries(fallbacks).flatMap(([from, targets]) => targets.map((to) => `${from}->${to}`)),
  );

  // (a) Config drift: when every class route exists, each class route's
  // route-typed targets must END with its declared fallback chain, in
  // order. Partial configs (unit fixtures) skip the layout checks.
  const classes = Object.keys(fallbacks);
  if (classes.every((name) => isObject(modelRoutes[name]))) {
    for (const name of classes) {
      const declared = fallbacks[name];
      const ids = routeTargetIds(modelRoutes[name], modelRoutes);
      const tail = ids.slice(ids.length - declared.length);
      if (JSON.stringify(tail) !== JSON.stringify(declared)) {
        issues.push(
          `fallback: route "${name}" must end with its class fallback chain [${declared.join(', ')}] as its final route targets (found route targets [${ids.join(', ')}])`,
        );
      }
    }

    // (b) Reachability: every class can reach specialist, the sink,
    // through the fallback graph.
    for (const name of classes) {
      const seen = new Set([name]);
      const queue = [name];
      while (queue.length > 0) {
        const current = queue.shift();
        for (const next of fallbacks[current] ?? []) {
          if (!seen.has(next)) {
            seen.add(next);
            queue.push(next);
          }
        }
      }
      if (!seen.has('specialist')) {
        issues.push(`fallback: class "${name}" cannot reach specialist through the fallback graph`);
      }
    }

    // (c) Escalation walk: simulate the router's depth-first walk of
    // the fallback chains under the per-call visited set. Entering a
    // class runs its regular chain, then its fallback edges in order;
    // a class already visited in this call is never re-entered, and
    // when a fallback subtree yields nothing the walk resumes the
    // caller's remaining edges (exactly how the appended targets sit
    // in each class route's target list). The walk must reach
    // specialist from every class.
    const walkReachesSpecialist = (start) => {
      const visited = new Set();
      const dfs = (current) => {
        if (current === 'specialist') return true;
        visited.add(current);
        for (const next of fallbacks[current] ?? []) {
          if (visited.has(next)) continue;
          if (dfs(next)) return true;
        }
        return false;
      };
      return dfs(start);
    };
    for (const name of classes) {
      if (!walkReachesSpecialist(name)) {
        issues.push(
          `fallback: escalation walk from "${name}" never reaches specialist under the per-call visited set`,
        );
      }
    }
  }

  // (d) Composition cycles: walk every route->route edge EXCEPT the
  // declared class fallback pairs. A cycle here is a real defect class
  // (nested composition must be a DAG); fallback pairs are governed by
  // the per-call visited set instead.
  const state = new Map(); // route -> 'open' | 'done'
  const stack = [];
  const visit = (routeId) => {
    state.set(routeId, 'open');
    stack.push(routeId);
    for (const next of routeTargetIds(modelRoutes[routeId], modelRoutes)) {
      if (fallbackPairs.has(`${routeId}->${next}`)) continue;
      if (state.get(next) === 'open') {
        const cycle = [...stack.slice(stack.indexOf(next)), next];
        if (!reportedCycleKeys.has(cycleKey(cycle))) {
          issues.push(`route composition cycle: ${cycle.join(' -> ')}`);
          reportedCycleKeys.add(cycleKey(cycle));
        }
        continue;
      }
      if (!state.has(next)) visit(next);
    }
    stack.pop();
    state.set(routeId, 'done');
  };
  for (const routeId of Object.keys(modelRoutes).sort()) {
    if (!state.has(routeId)) visit(routeId);
  }

  return issues;
}
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
        // Re-entry through a declared class fallback edge (e.g.
        // workhorse -> scout -> workhorse) is the designed per-call
        // visited-set semantics, not a cycle: the runtime expander
        // prunes the re-entry. The traversal itself is bounded by
        // the seen set below either way.
        if (!CLASS_FALLBACK_PAIRS.has(`${stack.at(-1)}->${parsed.model}`)) {
          addIssue(`${tier}: model-route cycle ${[...stack, parsed.model].join(' -> ')}`);
        }
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

  // Class fallback layout + whole-graph composition cycle pass.
  // Cycles the tier resolver already reported are not repeated.
  const reportedCycleKeys = new Set();
  for (const issue of issues) {
    const match = /model-route cycle (.+)$/.exec(issue);
    if (match) reportedCycleKeys.add(cycleKey(match[1].split(' -> ')));
  }
  for (const issue of inspectFallbackLayout(modelRoutes, CLASS_FALLBACKS, reportedCycleKeys)) {
    addIssue(issue);
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
