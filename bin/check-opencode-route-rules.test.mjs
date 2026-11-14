import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { checkRouteRules, inspectRouteRules } from './check-opencode-route-rules.mjs';

function configWith(targets = {}) {
  const tierTargets = {
    scout: ['good/model#low'],
    workhorse: ['good/model#high'],
    specialist: ['opencode-route/remapped#low'],
    summit: ['plan/model#high'],
    ...targets,
  };
  return {
    disabled_providers: ['disabled'],
    provider: {
      good: { models: { model: { variants: { low: {}, high: {} } } } },
      disabled: { models: { model: { variants: { low: {}, high: {} } } } },
      deepseek: { models: { ds: { variants: { low: {} } } } },
    },
    experimental: {
      provider_routes: { plan: { providers: ['good', 'disabled'] } },
      model_routes: {
        remapped: { targets: [{ model: 'good/model', variants: { low: 'high' } }] },
        ...Object.fromEntries(Object.entries(tierTargets).map(([tier, tierRouteTargets]) => [
          tier,
          { targets: tierRouteTargets },
        ])),
      },
    },
  };
}

test('resolves nested model/provider routes and applies effort remaps', () => {
  const result = inspectRouteRules(configWith());
  assert.deepEqual(result.issues, []);
  assert.deepEqual(
    result.tiers.specialist.coverage.map(({ provider, model, effort }) => ({ provider, model, effort })),
    [{ provider: 'good', model: 'model', effort: 'high' }],
  );
  assert.deepEqual(
    result.tiers.summit.coverage.map(({ provider, model, effort }) => ({ provider, model, effort })),
    [{ provider: 'good', model: 'model', effort: 'high' }],
  );
});

test('inline effort pins override a target remap and top-level route maps are fallback inputs', () => {
  const config = configWith({ specialist: [{ model: 'good/model#low', variants: { low: 'high' } }] });
  config.model_routes = config.experimental.model_routes;
  config.provider_routes = config.experimental.provider_routes;
  delete config.experimental.model_routes;
  delete config.experimental.provider_routes;

  const result = inspectRouteRules(config);
  assert.deepEqual(result.issues, []);
  assert.deepEqual(
    result.tiers.specialist.coverage.map(({ effort }) => effort),
    ['low'],
  );
});

test('disabled provider members are excluded from effective pools and coverage', () => {
  const result = inspectRouteRules(configWith({ scout: ['plan/model#low'] }));
  assert.deepEqual(result.issues, []);
  assert.equal(result.tiers.scout.disabled_excluded.length, 1);
  assert.ok(result.tiers.scout.members.every((candidate) => candidate.provider !== 'disabled'));
  assert.ok(result.tiers.scout.coverage.every((candidate) => candidate.provider !== 'disabled'));
});

test('a disabled provider-route group is excluded before member expansion', () => {
  const config = configWith({ scout: ['plan/model#low'] });
  config.disabled_providers.push('plan');
  const result = inspectRouteRules(config);
  assert.equal(result.tiers.scout.coverage.length, 0);
  assert.equal(result.tiers.scout.members.length, 0);
  assert.equal(result.tiers.scout.disabled_excluded.length, 1);
  assert.equal(result.tiers.scout.disabled_excluded[0].disabled_route, true);
  assert.ok(result.issues.some((issue) => issue.includes('no enabled, tracked, existing, non-funds model')));
});

test('provider-route expansion skips members whose tracked catalogs lack the target', () => {
  const config = configWith({ scout: ['plan/model#low'] });
  config.provider.other = { models: { different: {} } };
  config.experimental.provider_routes.plan.providers.push('other');
  const result = inspectRouteRules(config);
  assert.deepEqual(result.issues, []);
  assert.deepEqual(result.tiers.scout.coverage.map(({ provider }) => provider), ['good']);
});

test('provider-route members use concrete-provider precedence over same-named groups', () => {
  const config = configWith({ scout: ['plan/model#low'] });
  config.experimental.provider_routes.good = { providers: ['disabled'] };
  config.experimental.provider_routes.plan.providers = ['good'];
  const result = inspectRouteRules(config);
  assert.deepEqual(result.issues, []);
  assert.deepEqual(result.tiers.scout.coverage.map(({ provider }) => provider), ['good']);
});

test('unconfigured provider-route members are excluded from membership and effort checks', () => {
  const config = configWith({ scout: ['good/model', 'plan/model#low'] });
  config.experimental.provider_routes.plan.providers = ['typo'];
  const result = inspectRouteRules(config);
  assert.deepEqual(result.tiers.scout.members.map(({ provider }) => provider), ['good']);
  assert.equal(result.tiers.scout.effort_pinned, 0);
  assert.ok(result.issues.some((issue) => issue.includes('non-concrete member "typo"')));
  assert.ok(result.issues.some((issue) => issue.includes('good/model has no effort selected')));
});

test('a built-in concrete provider takes precedence over a same-named provider route', () => {
  const config = configWith({ scout: ['openai/model#low'] });
  config.experimental.provider_routes.openai = { providers: ['good'] };
  const result = inspectRouteRules(config);
  assert.equal(result.tiers.scout.coverage.length, 0);
  assert.equal(result.tiers.scout.members[0].provider, 'openai');
  assert.ok(result.issues.some((issue) => issue.startsWith('scout: no enabled, tracked, existing, non-funds model')));
});

test('disabled and funds-only candidates cannot satisfy tier coverage', () => {
  const config = configWith(Object.fromEntries(
    ['scout', 'workhorse', 'specialist', 'summit'].map((tier) => [
      tier,
      ['disabled/model#low', 'deepseek/ds#low'],
    ]),
  ));
  const result = inspectRouteRules(config);
  assert.equal(result.issues.filter((issue) => issue.includes('no enabled, tracked, existing, non-funds model')).length, 4);
  assert.equal(result.tiers.scout.coverage.length, 0);
  assert.equal(result.tiers.scout.disabled_excluded.length, 1);
  assert.equal(result.tiers.scout.funds_excluded.length, 1);
});

test('tracked model variants require an explicit supported effort', () => {
  const config = configWith(Object.fromEntries(
    ['scout', 'workhorse', 'specialist', 'summit'].map((tier) => [tier, ['good/model']]),
  ));
  const issues = checkRouteRules(config);
  assert.equal(issues.filter((issue) => issue.includes('has no effort selected')).length, 4);

  for (const tier of ['scout', 'workhorse', 'specialist', 'summit']) {
    config.experimental.model_routes[tier].targets = ['good/model#low'];
  }
  assert.deepEqual(checkRouteRules(config), []);
});

test('flags missing tracked models, unsupported variants, and route cycles', () => {
  const config = configWith({
    scout: ['good/missing#low'],
    workhorse: ['good/model#max'],
    specialist: ['opencode-route/loop#low'],
    summit: ['good/model#high'],
  });
  config.experimental.model_routes.loop = { targets: ['opencode-route/loop'] };
  const issues = checkRouteRules(config);
  assert.ok(issues.some((issue) => issue.includes('good/missing is missing from its tracked provider catalog')));
  assert.ok(issues.some((issue) => issue.includes('good/model#max is not a declared variant')));
  assert.ok(issues.some((issue) => issue.includes('model-route cycle')));
});

test('follows instanceOf for tracked models and their effort variants', () => {
  const config = configWith({ scout: ['alias/model#low'] });
  config.provider.alias = { instanceOf: 'good' };
  const result = inspectRouteRules(config);
  assert.deepEqual(result.issues, []);
  assert.equal(result.tiers.scout.coverage[0].catalog, 'tracked');
  assert.equal(result.tiers.scout.effort_variants_checked, 1);
});

test('unknown non-inverted window gates are rejected and excluded from coverage', () => {
  const config = configWith({ scout: [{ model: 'good/model#low', window: 'typo-window' }] });
  config.experimental.model_route_windows = {};
  const result = inspectRouteRules(config);
  assert.equal(result.tiers.scout.coverage.length, 0);
  assert.ok(result.issues.some((issue) => issue.includes('unknown model-route window "typo-window"')));
});

test('permanently closed window gates are rejected and excluded from coverage', () => {
  const config = configWith({ scout: [{ model: 'good/model#low', window: 'closed' }] });
  config.experimental.model_route_windows = {
    closed: { start: '25:00', end: '02:00', timezone: 'UTC' },
  };
  const result = inspectRouteRules(config);
  assert.equal(result.tiers.scout.coverage.length, 0);
  assert.ok(result.issues.some((issue) => issue.includes('permanently closed model-route window "closed"')));
});

test('repeated nested-route references expand once like the runtime', () => {
  const config = configWith({ scout: ['opencode-route/shared#low', 'opencode-route/shared#max'] });
  config.experimental.model_routes.shared = { targets: ['good/model'] };
  const result = inspectRouteRules(config);
  // The runtime shares one seen set per resolution, so the second
  // reference to `shared` is skipped and its `#max` variant (which the
  // tracked catalog does not declare) is never dispatched or judged.
  assert.ok(
    !result.issues.some((issue) => issue.startsWith('scout:') && issue.includes('not a declared variant')),
    `unexpected scout variant issue: ${result.issues}`,
  );
  assert.deepEqual(
    result.tiers.scout.coverage.map(({ effort }) => effort),
    ['low'],
  );
  assert.equal(result.tiers.scout.candidate_count, 1);
});

test('an emptied chain stage contributes nothing instead of failing the tier', () => {
  // main's workhorse-primary is deliberately targets: [] while the
  // chain's other stages serve the tier (Drew emptied it 2026-11-07).
  const config = configWith({
    workhorse: ['opencode-route/empty-stage#low', 'good/model#high'],
  });
  config.experimental.model_routes['empty-stage'] = { targets: [] };
  const result = inspectRouteRules(config);
  assert.ok(
    !result.issues.some((issue) => issue.includes('has no targets')),
    `emptied stage must not be an issue: ${result.issues}`,
  );
  assert.deepEqual(result.tiers.workhorse.coverage.map(({ provider }) => provider), ['good']);
});

test('a route with no targets list at all is still flagged', () => {
  const config = configWith({ scout: ['opencode-route/targetless#low'] });
  config.experimental.model_routes.targetless = {};
  const issues = checkRouteRules(config);
  assert.ok(issues.some((issue) => issue.includes('has no targets list')));
});

test('canonical OpenCode config satisfies the route rules', () => {
  const source = fs.readFileSync('.opencode/opencode.jsonc', 'utf8');
  assert.deepEqual(checkRouteRules(source), []);
});

// --- class fallback layout (Drew 2026-11-13) ---
import { CLASS_FALLBACKS, inspectFallbackLayout } from './check-opencode-route-rules.mjs';

function configWithFallbacks(overrides = {}) {
  const config = configWith({
    scout: ['good/model#low', 'opencode-route/workhorse'],
    workhorse: ['good/model#high', 'opencode-route/scout', 'opencode-route/specialist'],
    summit: ['plan/model#high', 'opencode-route/specialist'],
    ...overrides,
  });
  const routes = config.experimental.model_routes;
  routes.small = { targets: ['good/model#low', 'opencode-route/speed'] };
  routes.speed = {
    targets: ['good/model#high', 'opencode-route/workhorse', 'opencode-route/summit', 'opencode-route/specialist'],
  };
  routes.background = { targets: ['good/model#low', 'opencode-route/workhorse'] };
  return config;
}

test('class fallback layout passes and the workhorse<->scout pair is not a cycle', () => {
  const result = inspectRouteRules(configWithFallbacks());
  assert.deepEqual(result.issues, []);
});

test('class fallback drift between config and the declared chain is flagged', () => {
  const config = configWithFallbacks({
    workhorse: ['good/model#high', 'opencode-route/scout'],
  });
  const issues = inspectRouteRules(config).issues;
  assert.ok(
    issues.some((issue) => issue.includes('fallback') && issue.includes('"workhorse"') && issue.includes('class fallback chain')),
    `expected a workhorse fallback drift issue, got: ${JSON.stringify(issues)}`,
  );
});

test('a composition cycle between non-class routes is flagged with its path', () => {
  const config = configWithFallbacks();
  config.experimental.model_routes.cycA = { targets: ['opencode-route/cycB'] };
  config.experimental.model_routes.cycB = { targets: ['opencode-route/cycA'] };
  const issues = inspectRouteRules(config).issues;
  assert.ok(
    issues.some((issue) => issue.includes('route composition cycle: cycA -> cycB -> cycA')),
    `expected a composition cycle issue, got: ${JSON.stringify(issues)}`,
  );
});

test('fallback reachability check fires when a class cannot reach specialist', () => {
  const routes = configWithFallbacks().experimental.model_routes;
  const issues = inspectFallbackLayout(routes, { ...CLASS_FALLBACKS, summit: [] });
  assert.ok(
    issues.some((issue) => issue.includes('"summit"') && issue.includes('cannot reach specialist')),
    `expected a reachability issue, got: ${JSON.stringify(issues)}`,
  );
});

test('fallback escalation walk fires when the chain loops without reaching specialist', () => {
  const routes = configWithFallbacks().experimental.model_routes;
  const issues = inspectFallbackLayout(routes, { ...CLASS_FALLBACKS, speed: ['summit'], summit: ['speed'] });
  assert.ok(
    issues.some((issue) => issue.includes('escalation walk') && issue.includes('never reaches specialist')),
    `expected an escalation walk issue, got: ${JSON.stringify(issues)}`,
  );
});
