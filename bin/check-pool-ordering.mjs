#!/usr/bin/env node

// Pool-ordering audit for the daily model + route audit (#1173). It checks
// the one ordering rule that is decidable from the tracked config alone —
// OpenAI as final reserve — and, when the #1270 plan store is passed with
// --plans, reports the burn-by-expiry order per tier (each pool's quota
// window end, derived from the store) so the daily audit can compare it
// against the config order instead of by hand. Cheapest-sufficient is
// still unwired: the store's fee is a subscription price, not a cost per
// unit of work.
//
// Rule (decided 2026-11-22, xaltha; see the decision record on #1173):
// in every ordered chain, an OpenAI-backed pool must come last. Grounded
// in the config's own words: "Sol #xhigh moves LAST. The OpenAI plan is
// the fleet's deepest reserve, so the chain spends Antigravity (Opus),
// the Go contributor bucket (Argon), and Meta (Wonder) before it touches
// OpenAI at all" (.opencode/opencode.jsonc, summit chain), and "the
// ChatGPT doors stay at the default as the OpenAI reserve" (luna-plan).
// An "OpenAI-backed pool" is a model route whose doors include the
// OpenAI plan (the openai-plan provider route / _openai.* doors). A leaf
// serving an OpenAI model through another provider's plan
// (opendesign/gpt-6.1-sol) is that provider's door, not the OpenAI
// plan's, and does not count as OpenAI here.
//
// Burn-by-expiry derivation (decided 2026-11-23, xaltha; #1173/#1149):
// a plan quota with resetRule "calendar" and windowHours W is the
// calendar period of length W containing now, so its window ends at the
// start of the next such period, in UTC: 24h ends at the next midnight,
// 168h at the next Monday 00:00 (ISO week), 720h at the 1st of the next
// month. Any other window length, a missing window, or a non-"calendar"
// resetRule has no derivable end and is reported unknown, never guessed.
// A pool's expiry is the earliest window end among the plans covering
// any of its leaf providers; a plan with status present and not "active"
// does not bind. Pool-to-plan matching normalises provider ids (drop a
// leading "_", drop a "." alias suffix, drop "-" separators,
// lowercase), so the store's "open-design" matches the config's
// "opendesign" and "claude" matches "_claude". A pool with no covering plan, or only
// underivable windows, is reported unknown. Expiry-order mismatches are
// notes, not findings, until the derivation is validated against the
// live store's actual reset instants (follow-up on #1173); the report
// names the exact pools and dates so the audit can act on them.
//
// Scope notes, deliberately:
// - Only ordered chains are order-checked. In a weighted or round-robin
//   route list position is not burn order (the route proxy rotates or
//   draws by weight; anything else keeps listed order), so a pool reached
//   through such a route is unordered-position. An OpenAI-backed pool
//   sharing an unordered route with non-OpenAI pools is a note (no
//   reserve encoding is visible), never a finding.
// - Fallback targets are separate passes: a tier's regular chain runs
//   first and completely, and each fallback target is a second pass that
//   engages only on exhaustion, so the reserve rule is checked within
//   each pass, not across the flat sequence. The split applies
//   transitively: a fallback target's own declared class fallbacks are
//   further passes, not a continuation of the fallback's chain.
// - Funds (metered) and disabled leaves never serve as pool quota and
//   are excluded from the order, matching inspectRouteRules.

// Argument contract (#3352): the command line is exactly one optional
// config path plus --json and --plans <plans.json>. Any other token —
// including a single-dash flag such as `-j` — is a usage error raised
// by parseArgs before the config is read, never a positional.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseJsonc } from './provider-catalog-sync.mjs';
import {
  CLASS_FALLBACKS,
  REQUIRED_TIERS,
  inspectRouteRules,
} from './check-opencode-route-rules.mjs';

export const DEFAULT_CONFIG = '.opencode/opencode.jsonc';

function isObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function activeMap(config, key) {
  const experimental = config.experimental;
  const preferred = experimental?.[key];
  if (isObject(preferred) || Array.isArray(preferred)) return preferred;
  return config[key];
}

// An OpenAI-plan door: a leaf served through the ChatGPT pair
// (openai-plan provider route, _openai.* doors) or the bare openai
// provider (including its retired openai.* / openai-* aliases, which
// are OpenAI doors if they ever seat again).
const OPENAI_PROVIDER_PREFIXES = ['openai.', 'openai-', '_openai'];
function isOpenAiLeaf(candidate) {
  if (typeof candidate.provider === 'string') {
    if (candidate.provider === 'openai') return true;
    if (OPENAI_PROVIDER_PREFIXES.some((prefix) => candidate.provider.startsWith(prefix))) {
      return true;
    }
  }
  return String(candidate.trace ?? '')
    .split(' -> ')
    .includes('openai-plan');
}

const TRACE_ROUTE = /^(.+)\.targets\[(\d+)\]$/;

// Split a candidate trace (e.g. "summit -> summit.targets[4] ->
// sol-plan.targets[0] -> openai-plan") into its route-expansion steps.
function traceSegments(trace) {
  const segments = [];
  for (const part of String(trace ?? '').split(' -> ')) {
    const match = TRACE_ROUTE.exec(part);
    if (match) segments.push({ route: match[1], index: Number(match[2]) });
  }
  return segments;
}

// The model reference of a route target, or null when the target is not
// an opencode-route reference.
function routeTargetName(target) {
  const reference =
    typeof target === 'string' ? target : isObject(target) ? target.model : null;
  if (typeof reference !== 'string') return null;
  const hash = reference.lastIndexOf('#');
  const bare = hash < 0 ? reference : reference.slice(0, hash);
  const slash = bare.indexOf('/');
  if (slash < 0 || bare.slice(0, slash) !== 'opencode-route') return null;
  return bare.slice(slash + 1) || null;
}

// How many trailing targets of a tier route are its declared class
// fallbacks (longest suffix of the target list that is a suffix of the
// tier's CLASS_FALLBACKS chain). Those targets are separate passes.
function fallbackTailLength(tierRoute, tier) {
  const fallbacks = CLASS_FALLBACKS[tier] ?? [];
  if (fallbacks.length === 0 || !Array.isArray(tierRoute?.targets)) return 0;
  const names = tierRoute.targets.map(routeTargetName);
  let tail = 0;
  while (
    tail < fallbacks.length &&
    tail < names.length &&
    names[names.length - 1 - tail] === fallbacks[fallbacks.length - 1 - tail]
  ) {
    tail += 1;
  }
  return tail;
}

// The pool a leaf was reached through: the innermost model route in its
// trace. A leaf with no route beyond the tier itself is a direct door
// and is its own singleton pool, identified by its provider/model door —
// two direct leaves from the same provider but different models are
// different pools, so a trailing non-OpenAI seat cannot collapse into
// an earlier one and disappear from the check.
function leafPool(candidate, tier) {
  const segments = traceSegments(candidate.trace);
  for (let i = segments.length - 1; i >= 1; i -= 1) {
    if (segments[i].route !== tier) return segments[i].route;
  }
  return `direct:${candidate.provider}/${candidate.model}`;
}

// The route chain governing a pool's position: the tier route plus every
// intermediate route on the leaf's trace, excluding the pool's own
// route. A pool is ordered-position only when every governing route
// keeps listed order (selection "ordered" or absent — the route proxy
// rotates only on round-robin and draws by weight only on weighted).
function leafAncestors(candidate, tier) {
  const segments = traceSegments(candidate.trace);
  const ancestors = [];
  for (let i = 0; i < segments.length; i += 1) {
    const route = segments[i].route;
    if (i > 0 && route !== tier && route === leafPool(candidate, tier)) break;
    ancestors.push(route);
  }
  return ancestors;
}

function selectionOf(modelRoutes, route) {
  const selection = modelRoutes?.[route]?.selection;
  return typeof selection === 'string' ? selection : 'ordered';
}

// --- Burn-by-expiry (--plans) -------------------------------------------
// The decided source is the #1270 plan store (plans.json): the same
// shape ~/tools/provider-usage/reconcile.py reads — plans[].id,
// plans[].providerID, plans[].fee{amount,period},
// plans[].quotas[]{windowHours,limit,resetRule}, plans[].status.

// The end of the calendar period of length windowHours containing now,
// in UTC ms; null when no end is derivable (see the header comment).
// 24h is the UTC day, 168h the ISO week (Monday 00:00 UTC), 720h the
// calendar month. A quota whose window has already ended is impossible
// by construction: the period containing now always ends in the future.
export function windowEndForQuota(quota, now = Date.now()) {
  if (!isObject(quota)) return null;
  if (quota.resetRule !== 'calendar') return null;
  const hours = quota.windowHours;
  if (typeof hours !== 'number' || !Number.isFinite(hours)) return null;
  const at = new Date(now);
  if (hours === 24) {
    return Date.UTC(at.getUTCFullYear(), at.getUTCMonth(), at.getUTCDate() + 1);
  }
  if (hours === 168) {
    // ISO week: Monday 00:00 UTC. getUTCDay: 0 = Sunday .. 6 = Saturday.
    const day = at.getUTCDay();
    const daysToMonday = (day + 6) % 7;
    const monday = Date.UTC(at.getUTCFullYear(), at.getUTCMonth(), at.getUTCDate() - daysToMonday);
    return monday + 7 * 24 * 3600 * 1000;
  }
  if (hours === 720) {
    return Date.UTC(at.getUTCFullYear(), at.getUTCMonth() + 1, 1);
  }
  return null;
}

// Normalise a provider id for pool-to-plan matching: the store writes
// bare ids ("open-design", "claude") while the config seats underscored
// aliases ("opendesign", "_claude", "_openai.alt"). A leading "_" is
// dropped, a "." suffix is an alias qualifier ("_openai.alt" is the
// openai provider's alt door), and "-" separators are dropped.
export function normaliseProviderId(provider) {
  return String(provider ?? '')
    .replace(/^_+/, '')
    .split('.')[0]
    .replace(/[-_]/g, '')
    .toLowerCase();
}

// Normalised plan rows from a parsed plans.json store: one row per
// plan with a derivable window end, keyed by normalised provider.
// Plans with status present and not "active" do not bind; a plan with
// several quotas binds at its earliest-ending one (the binding
// constraint for burn order). Malformed input yields no rows, never a
// throw: the caller reports the pools as unknown.
export function planWindowEnds(store, now = Date.now()) {
  const byProvider = new Map();
  const plans = isObject(store) && Array.isArray(store.plans) ? store.plans : [];
  for (const plan of plans) {
    if (!isObject(plan)) continue;
    if (plan.status !== undefined && plan.status !== 'active') continue;
    const provider = normaliseProviderId(plan.providerID);
    if (!provider) continue;
    const quotas = Array.isArray(plan.quotas) ? plan.quotas : [];
    let end = null;
    for (const quota of quotas) {
      const candidate = windowEndForQuota(quota, now);
      if (candidate !== null && (end === null || candidate < end)) end = candidate;
    }
    if (end === null) continue;
    const key = provider;
    if (!byProvider.has(key) || end < byProvider.get(key).windowEndMs) {
      byProvider.set(key, { plan: plan.id ?? plan.plan ?? null, provider: plan.providerID, windowEndMs: end });
    }
  }
  return byProvider;
}

// A pool's quota window end: the earliest end among the plans covering
// any of its leaf providers, or null when no provider is covered. The
// providers not covered are returned so the report can name them.
export function poolWindowEnd(providers, plansByProvider) {
  let end = null;
  const unknown = [];
  for (const provider of providers) {
    const row = plansByProvider.get(normaliseProviderId(provider));
    if (!row) {
      unknown.push(provider);
      continue;
    }
    if (end === null || row.windowEndMs < end) end = row.windowEndMs;
  }
  return { windowEndMs: end, unknownProviders: unknown };
}

export function inspectPoolOrdering(config, options = {}) {
  const plansByProvider =
    options.plans === undefined || options.plans === null
      ? null
      : planWindowEnds(options.plans, options.now ?? Date.now());
  const issues = [];
  const notes = [];
  const noteSet = new Set();
  const addNote = (note) => {
    if (!noteSet.has(note)) {
      noteSet.add(note);
      notes.push(note);
    }
  };
  const routeRules = inspectRouteRules(config);
  const modelRoutes = activeMap(config, 'model_routes') ?? {};
  const tiers = {};
  let expiry = null;

  for (const tier of REQUIRED_TIERS) {
    const resolved = routeRules.tiers[tier];
    const members = resolved?.members ?? [];
    const eligible = members.filter((candidate) => !candidate.funds);

    // A pool is OpenAI-backed when any of its (non-disabled) leaves is
    // an OpenAI-plan door. Funds leaves count for classification — they
    // are still the pool's doors — but never for ordering.
    const openaiBacked = {};
    for (const candidate of members) {
      const pool = leafPool(candidate, tier);
      if (isOpenAiLeaf(candidate)) openaiBacked[pool] = true;
      else if (!(pool in openaiBacked)) openaiBacked[pool] = false;
    }

    // The leaf providers behind each pool, from its serving (non-funds)
    // leaves: the providers whose plans bound the pool's quota window.
    // A direct pool's provider is its own door; a route pool draws from
    // every provider seated under it.
    const providersByPool = new Map();
    for (const candidate of eligible) {
      const pool = leafPool(candidate, tier);
      if (!providersByPool.has(pool)) providersByPool.set(pool, new Set());
      if (typeof candidate.provider === 'string') {
        providersByPool.get(pool).add(candidate.provider);
      }
    }

    // A leaf's pass is the sequence of fallback hops on its trace: each
    // step into a target that sits in its route's declared
    // class-fallback tail is a separate pass (that target runs only on
    // exhaustion of the route's regular chain), and the split applies
    // transitively — a fallback target's own fallback targets are
    // further passes, not a continuation of the fallback's chain. Steps
    // into main-chain targets stay in the current pass, so direct leaves
    // of a route and leaves via its main-chain nested routes share one
    // pass. A pass with no leaves is not reported.
    const fallbackHops = (candidate) => {
      const hops = [];
      for (const segment of traceSegments(candidate.trace)) {
        const route = isObject(modelRoutes) ? modelRoutes[segment.route] : null;
        const tail = fallbackTailLength(route, segment.route);
        const targetCount = Array.isArray(route?.targets) ? route.targets.length : 0;
        if (tail > 0 && segment.index >= Math.max(0, targetCount - tail)) {
          hops.push(
            routeTargetName(route.targets[segment.index]) ?? `targets[${segment.index}]`,
          );
        }
      }
      return hops;
    };

    // Per-leaf facts, grouped by pass (first appearance = chain order).
    const leavesByPass = new Map();
    for (const candidate of eligible) {
      const segments = traceSegments(candidate.trace);
      if (segments.length === 0) continue;
      const hops = fallbackHops(candidate);
      const label = hops.length === 0 ? 'main chain' : `fallback via ${hops.join(' -> ')}`;
      if (!leavesByPass.has(label)) leavesByPass.set(label, []);
      const pool = leafPool(candidate, tier);
      const ancestors = leafAncestors(candidate, tier);
      const unorderedAncestor = [...ancestors]
        .reverse()
        .find((route) => selectionOf(modelRoutes, route) !== 'ordered');
      leavesByPass.get(label).push({
        pool,
        ordered: unorderedAncestor === undefined,
        unorderedAncestor,
        openai: isOpenAiLeaf(candidate),
      });
    }

    // Which pools each route governs (for the mixing notes): every pool
    // reached through that route, in this tier. A route's direct leaves
    // make the route itself a pool in the order, so a pool is also
    // recorded under its own name.
    const poolsByRoute = new Map();
    const recordPoolUnderRoute = (pool, route) => {
      if (!poolsByRoute.has(route)) poolsByRoute.set(route, new Set());
      poolsByRoute.get(route).add(pool);
    };
    for (const candidate of eligible) {
      const pool = leafPool(candidate, tier);
      for (const route of leafAncestors(candidate, tier)) {
        recordPoolUnderRoute(pool, route);
      }
      if (!pool.startsWith('direct:')) recordPoolUnderRoute(pool, pool);
    }

    const passes = [];
    const expiryPasses = [];
    let tierReserve = 'n/a';
    for (const [label, passLeaves] of leavesByPass) {
      const pools = [];
      const ordered = {};
      const backed = {};
      for (const leaf of passLeaves) {
        if (!pools.includes(leaf.pool)) pools.push(leaf.pool);
        ordered[leaf.pool] = leaf.ordered && (ordered[leaf.pool] ?? true);
        backed[leaf.pool] = openaiBacked[leaf.pool] ?? false;
      }
      const orderedSeq = pools.filter((pool) => ordered[pool]);
      const firstOpenAi = orderedSeq.findIndex((pool) => backed[pool]);
      let reserve = 'n/a';
      if (firstOpenAi >= 0) {
        reserve = 'pass';
        const later = orderedSeq
          .slice(firstOpenAi + 1)
          .find((pool) => !backed[pool]);
        if (later) {
          reserve = 'violation';
          issues.push(
            `${tier} ${label}: OpenAI-backed pool "${orderedSeq[firstOpenAi]}" ` +
              `is tried before non-OpenAI pool "${later}" ` +
              `(pool order: [${orderedSeq.join(', ')}])`,
          );
        }
      }
      if (reserve === 'violation') tierReserve = 'violation';
      else if (reserve === 'pass' && tierReserve !== 'violation') tierReserve = 'pass';
      passes.push({
        label,
        pools,
        ordered,
        openai_backed: backed,
        reserve,
      });

      // Burn-by-expiry (only with --plans): each pool's quota window
      // end, the expiry order for the pass, and a note when the config
      // order burns a later-expiring ordered pool before an
      // earlier-expiring one. Unordered pools have no burn order and
      // never contribute; unknown pools sort last in config order.
      // Notes, not findings, until the derivation is validated against
      // the live store's actual reset instants (#1173 follow-up).
      if (plansByProvider !== null) {
        const expiry = {};
        const unknown = [];
        const uncovered = new Set();
        for (const pool of pools) {
          const providers = [...(providersByPool.get(pool) ?? [])];
          const { windowEndMs, unknownProviders } = poolWindowEnd(providers, plansByProvider);
          expiry[pool] = windowEndMs === null ? null : new Date(windowEndMs).toISOString();
          if (windowEndMs === null) unknown.push(pool);
          for (const provider of unknownProviders) uncovered.add(provider);
        }
        const known = pools.filter((pool) => expiry[pool] !== null);
        const expiryOrder = [...pools].sort((a, b) => {
          if (expiry[a] === null && expiry[b] === null) return pools.indexOf(a) - pools.indexOf(b);
          if (expiry[a] === null) return 1;
          if (expiry[b] === null) return -1;
          return expiry[a] < expiry[b] ? -1 : expiry[a] > expiry[b] ? 1 : pools.indexOf(a) - pools.indexOf(b);
        });
        expiryPasses.push({ label, pools, expiry, expiryOrder, unknown });
        const orderedKnown = orderedSeq.filter((pool) => expiry[pool] !== null);
        let inversion = null;
        for (let i = 0; i < orderedKnown.length && !inversion; i += 1) {
          for (let j = i + 1; j < orderedKnown.length; j += 1) {
            if (expiry[orderedKnown[i]] > expiry[orderedKnown[j]]) {
              inversion = { later: orderedKnown[i], earlier: orderedKnown[j] };
              break;
            }
          }
        }
        if (inversion) {
          addNote(
            `${tier} ${label}: burn-by-expiry order differs from config order — ` +
              `"${inversion.earlier}" expires ${expiry[inversion.earlier]} but is tried after ` +
              `"${inversion.later}" (expires ${expiry[inversion.later]})`,
          );
        }
        if (uncovered.size > 0) {
          addNote(
            `${tier} ${label}: no plan-store expiry for provider(s) ${[...uncovered].sort().join(', ')} ` +
              `— those pools are unknown in the expiry order`,
          );
        }
      }

      // Mixing notes: an OpenAI-backed pool sharing an unordered route
      // with non-OpenAI pools has no reserve encoding visible there.
      for (const leaf of passLeaves) {
        if (!leaf.openai || leaf.ordered || !leaf.unorderedAncestor) continue;
        const route = leaf.unorderedAncestor;
        const sharers = poolsByRoute.get(route) ?? new Set();
        const mixes = [...sharers].some(
          (pool) => pool !== leaf.pool && !openaiBacked[pool],
        );
        if (!mixes) continue;
        addNote(
          `${tier} ${label}: ${leaf.pool} shares ${route} ` +
            `(selection ${selectionOf(modelRoutes, route)}) with non-OpenAI pools ` +
            `at equal/default weight — no reserve encoding is visible` +
            (plansByProvider === null
              ? `; burn-by-expiry and cheapest-sufficient need the #1270 plan store`
              : `; cheapest-sufficient is still unwired (the store's fee is a subscription price, not a cost per unit of work)`),
        );
      }
    }

    if (eligible.length === 0) {
      addNote(`${tier}: no eligible leaves; pool ordering not checked`);
    }
    tiers[tier] = { passes, reserve: tierReserve };
    if (plansByProvider !== null) {
      if (expiry === null) expiry = {};
      expiry[tier] = { passes: expiryPasses };
    }
  }

  return { tiers, issues, notes, expiry };
}

export function checkPoolOrdering(source) {
  return inspectPoolOrdering(typeof source === 'string' ? parseJsonc(source) : source)
    .issues;
}

export function parseArgs(argv) {
  let configPath = null;
  let plansPath = null;
  let json = false;
  for (let i = 0; i < argv.length; i += 1) {
    const token = argv[i];
    if (token === '--json') {
      if (json) throw new Error('--json was given more than once');
      json = true;
    } else if (token === '--plans') {
      if (plansPath) throw new Error('--plans was given more than once');
      const value = argv[i + 1];
      if (value === undefined || value.startsWith('-')) {
        throw new Error('--plans needs a plans.json path');
      }
      plansPath = value;
      i += 1;
    } else if (token.startsWith('-')) {
      // Any dash-prefixed token that is not a declared flag is a usage
      // error, including single-dash flags such as `-j`: without this
      // they fall through to the config-path positional and the checker
      // silently audits whatever path they name (#3352).
      throw new Error(`unrecognised argument: ${token}`);
    } else if (configPath) {
      throw new Error(`unexpected positional argument: ${token}`);
    } else {
      configPath = token;
    }
  }
  return {
    configPath: path.resolve(configPath ?? DEFAULT_CONFIG),
    plansPath: plansPath === null ? null : path.resolve(plansPath),
    json,
  };
}

async function main(argv) {
  const { configPath, plansPath, json } = parseArgs(argv);
  const plans =
    plansPath === null ? null : JSON.parse(fs.readFileSync(plansPath, 'utf8'));
  const result = inspectPoolOrdering(parseJsonc(fs.readFileSync(configPath, 'utf8')), {
    plans,
  });
  if (json) {
    console.log(JSON.stringify({ config: configPath, plans: plansPath, ...result }, null, 2));
  } else {
    for (const tier of REQUIRED_TIERS) {
      const summary = result.tiers[tier];
      for (const pass of summary.passes) {
        console.log(
          `${tier} ${pass.label}: pools [${pass.pools.join(', ')}] (reserve: ${pass.reserve})`,
        );
      }
      if (summary.passes.length === 0) {
        console.log(`${tier}: no passes (reserve: ${summary.reserve})`);
      }
      const expiryPasses = result.expiry?.[tier]?.passes ?? [];
      for (const pass of expiryPasses) {
        const dated = pass.pools.map(
          (pool) => `${pool}@${pass.expiry[pool] ?? 'unknown'}`,
        );
        console.log(
          `${tier} ${pass.label}: expiry order [${pass.expiryOrder.join(', ')}] (${dated.join(', ')})`,
        );
      }
    }
    for (const note of result.notes) console.log(`note: ${note}`);
    for (const issue of result.issues) console.error(`- ${issue}`);
    if (result.issues.length === 0) {
      console.log(`Pool-ordering guard passed: ${configPath}`);
    }
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
    console.error(`[pool-ordering] ${error?.message ?? error}`);
    process.exitCode = 2;
  }
}
