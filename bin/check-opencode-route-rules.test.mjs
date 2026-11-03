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

test('canonical OpenCode config satisfies the route rules', () => {
  const source = fs.readFileSync('.opencode/opencode.jsonc', 'utf8');
  assert.deepEqual(checkRouteRules(source), []);
});
