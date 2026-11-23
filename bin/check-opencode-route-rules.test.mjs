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
    // Scout chain ruled by Drew 2026-11-14: background first, then workhorse
    // (matches CLASS_FALLBACKS, .opencode/route-fallbacks.md, and the canonical config).
    scout: ['good/model#low', 'opencode-route/background', 'opencode-route/workhorse'],
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
// --- purpose designations (code#13675, 2026-11-23) ---

function configWithDesignations() {
  const config = configWith();
  config.model_designations = {
    'decision-bot': { purposes: ['decision'] },
    'embed-bot': { purposes: ['embed'] },
    'stt-bot': { purposes: ['transcribe'], capabilities: ['stt', 'audio-in'] },
  };
  const routes = config.experimental.model_routes;
  routes.deciders = { purpose: 'decision', targets: ['free/decision-bot#low'] };
  routes.embedders = { purpose: 'embed', targets: ['free/embed-bot#low'] };
  routes.transcribers = { purpose: 'transcribe', targets: ['free/stt-bot#low'] };
  return config;
}

test('designated models pass when seated only in pools of their purpose', () => {
  const result = inspectRouteRules(configWithDesignations());
  assert.deepEqual(result.issues, []);
});

test('designation rule fires when a designated model is seated in a pool of another purpose', () => {
  const config = configWithDesignations();
  config.experimental.model_routes.embedders.targets = ['free/decision-bot#low'];
  const issues = inspectRouteRules(config).issues;
  assert.ok(
    issues.some((issue) => issue.includes('designation:') && issue.includes('"decision-bot"') && issue.includes('"embedders"')),
    `expected a designation issue for decision-bot in embedders, got: ${JSON.stringify(issues)}`,
  );
});

test('a model designated without chat never resolves into a chat-purpose pool (systemone convention)', () => {
  const config = configWithDesignations();
  // scout is a chat-purpose tier (no purpose field = chat).
  config.experimental.model_routes.scout.targets = ['good/model#low', 'free/embed-bot#low'];
  const issues = inspectRouteRules(config).issues;
  assert.ok(
    issues.some((issue) => issue.includes('designation:') && issue.includes('"embed-bot"') && issue.includes('"scout"') && issue.includes('purpose "chat"')),
    `expected a designation issue for embed-bot in scout, got: ${JSON.stringify(issues)}`,
  );
});

test('designation rule follows provider-route expansion into pools', () => {
  const config = configWithDesignations();
  config.provider.good.models['decision-bot'] = { variants: { low: {} } };
  config.experimental.model_routes.deciders.targets = ['plan/decision-bot#low'];
  assert.deepEqual(inspectRouteRules(config).issues, []);
  config.experimental.model_routes.workhorse.targets = ['plan/decision-bot#low'];
  const issues = inspectRouteRules(config).issues;
  assert.ok(
    issues.some((issue) => issue.includes('designation:') && issue.includes('"decision-bot"') && issue.includes('"workhorse"')),
    `expected a designation issue for decision-bot in workhorse, got: ${JSON.stringify(issues)}`,
  );
});

test('unknown designation purposes and capabilities are flagged', () => {
  const config = configWithDesignations();
  config.model_designations['decision-bot'] = { purposes: ['decision', 'fly'] };
  config.model_designations['stt-bot'] = { purposes: ['transcribe'], capabilities: ['stt', 'wings'] };
  const issues = inspectRouteRules(config).issues;
  assert.ok(issues.some((issue) => issue.includes('unknown purpose "fly"')), `got: ${JSON.stringify(issues)}`);
  assert.ok(issues.some((issue) => issue.includes('unknown capability "wings"')), `got: ${JSON.stringify(issues)}`);
});

test('an unknown pool purpose is flagged', () => {
  const config = configWithDesignations();
  config.experimental.model_routes.deciders.purpose = 'teleport';
  const issues = inspectRouteRules(config).issues;
  assert.ok(
    issues.some((issue) => issue.includes('designation:') && issue.includes('"deciders"') && issue.includes('unknown purpose "teleport"')),
    `expected an unknown-purpose issue for deciders, got: ${JSON.stringify(issues)}`,
  );
});

test('designations may also live under experimental like the other route maps', () => {
  const config = configWithDesignations();
  config.experimental.model_designations = config.model_designations;
  delete config.model_designations;
  config.experimental.model_routes.scout.targets = ['good/model#low', 'free/decision-bot#low'];
  const issues = inspectRouteRules(config).issues;
  assert.ok(
    issues.some((issue) => issue.includes('designation:') && issue.includes('"decision-bot"') && issue.includes('"scout"')),
    `expected a designation issue via experimental.model_designations, got: ${JSON.stringify(issues)}`,
  );
});

test('designation source, when present, must be a non-empty string', () => {
  const config = configWithDesignations();
  config.model_designations['decision-bot'] = { purposes: ['decision'], source: 'models.dev' };
  assert.deepEqual(inspectRouteRules(config).issues, []);
  config.model_designations['decision-bot'] = { purposes: ['decision'], source: '' };
  const issues = inspectRouteRules(config).issues;
  assert.ok(
    issues.some((issue) => issue.includes('designation:') && issue.includes('"decision-bot"') && issue.includes('source must be a non-empty string')),
    `expected a source validation issue, got: ${JSON.stringify(issues)}`,
  );
  config.model_designations['decision-bot'] = { purposes: ['decision'], source: 42 };
  assert.ok(inspectRouteRules(config).issues.some((issue) => issue.includes('source must be a non-empty string')));
});

// --- wallScope declarations + free/paid alias invariants (code#13693) ---
import { WALL_SCOPES, inspectProviderWallRules } from './check-opencode-route-rules.mjs';

function configWithWallSplit() {
  const config = configWith();
  config.provider.paid = {
    models: { 'free-model': {}, 'paid-model': {} },
    options: { baseURL: 'https://paid.example/v1' },
  };
  config.provider['paid-free'] = {
    instanceOf: 'paid',
    wallScope: 'per-model',
    models: { 'free-model': {} },
  };
  config.provider.paid.wallScope = 'provider';
  config.experimental.model_routes['free-pool'] = { targets: ['paid-free/free-model'] };
  return config;
}

test('wallScope vocabulary is the closed set from the design', () => {
  assert.deepEqual(WALL_SCOPES, ['per-model', 'provider', 'account']);
});

test('a well-formed wall split passes the wall rules and the full checker', () => {
  const config = configWithWallSplit();
  assert.deepEqual(inspectProviderWallRules(config), []);
  assert.deepEqual(checkRouteRules(config), []);
});

test('an unknown wallScope value fails, naming the provider', () => {
  const config = configWithWallSplit();
  config.provider.groq = { wallScope: 'per-org', models: {} };
  const issues = inspectProviderWallRules(config);
  assert.ok(
    issues.some((issue) => issue.includes('"groq"') && issue.includes('"per-org"')),
    `expected a wallScope issue naming groq: ${issues}`,
  );
  assert.ok(checkRouteRules(config).some((issue) => issue.includes('wallScope')));
});

test('every closed-vocabulary wallScope value passes', () => {
  for (const scope of WALL_SCOPES) {
    const config = configWithWallSplit();
    config.provider.groq = { wallScope: scope, models: {} };
    assert.deepEqual(inspectProviderWallRules(config), [], );
  }
});

test('a -free alias declaring a differing credential fails, naming both ids', () => {
  const config = configWithWallSplit();
  config.provider['paid-free'].options = { apiKey: 'a-different-key' };
  const issues = inspectProviderWallRules(config);
  assert.ok(
    issues.some((issue) => issue.includes('"paid-free"') && issue.includes('"paid"') && issue.includes('options.apiKey')),
    `expected an alias credential issue naming both ids: ${issues}`,
  );
});

test('a -free alias declaring a differing baseURL fails, naming both ids', () => {
  const config = configWithWallSplit();
  config.provider['paid-free'].options = { baseURL: 'https://elsewhere.example/v1' };
  const issues = inspectProviderWallRules(config);
  assert.ok(
    issues.some((issue) => issue.includes('"paid-free"') && issue.includes('"paid"') && issue.includes('options.baseURL')),
    `expected an alias baseURL issue naming both ids: ${issues}`,
  );
});

test('a -free alias re-declaring the base value verbatim does not differ and passes', () => {
  const config = configWithWallSplit();
  config.provider['paid-free'].options = { baseURL: 'https://paid.example/v1' };
  assert.deepEqual(inspectProviderWallRules(config), []);
});

test('a free-alias model also seated under the paid base fails, naming the seat', () => {
  const config = configWithWallSplit();
  config.experimental.model_routes['paid-pool'] = { targets: ['paid/free-model'] };
  const issues = inspectProviderWallRules(config);
  assert.ok(
    issues.some(
      (issue) =>
        issue.includes('"free-model"') && issue.includes('"paid-free"') && issue.includes('"paid"') && issue.includes('paid-pool'),
    ),
    `expected a split invariant issue naming the seat: ${issues}`,
  );
  assert.ok(checkRouteRules(config).some((issue) => issue.includes('wallScope')));
});

test('a free-alias model seated under the paid base via weights alone fails', () => {
  const config = configWithWallSplit();
  config.experimental.model_routes['paid-pool'] = {
    targets: ['paid/paid-model'],
    weights: { 'paid/free-model': 1, 'paid/paid-model': 1 },
  };
  const issues = inspectProviderWallRules(config);
  assert.ok(
    issues.some((issue) => issue.includes('"free-model"') && issue.includes('weights')),
    `expected a split invariant issue naming the weights seat: ${issues}`,
  );
});

test('a same-model paid tail passes when the free seat outweighs it in the same route', () => {
  const config = configWithWallSplit();
  config.experimental.model_routes['mixed-pool'] = {
    targets: ['paid-free/free-model', 'paid/free-model#low'],
    weights: { 'paid-free/free-model': 3, 'paid/free-model': 0.1 },
  };
  assert.deepEqual(inspectProviderWallRules(config), []);
});

test('a same-model paid tail at equal weight to the free seat fails', () => {
  const config = configWithWallSplit();
  config.experimental.model_routes['mixed-pool'] = {
    targets: ['paid-free/free-model', 'paid/free-model#low'],
    weights: { 'paid-free/free-model': 1, 'paid/free-model': 1 },
  };
  const issues = inspectProviderWallRules(config);
  assert.ok(
    issues.some((issue) => issue.includes('"free-model"') && issue.includes('mixed-pool')),
    `expected a split invariant issue for the equal-weight tail: ${issues}`,
  );
});

test('a same-model paid tail outweighing the free seat fails', () => {
  const config = configWithWallSplit();
  config.experimental.model_routes['mixed-pool'] = {
    targets: ['paid-free/free-model', 'paid/free-model#low'],
    weights: { 'paid-free/free-model': 0.1, 'paid/free-model': 3 },
  };
  const issues = inspectProviderWallRules(config);
  assert.ok(
    issues.some((issue) => issue.includes('"free-model"') && issue.includes('mixed-pool')),
    `expected a split invariant issue for the heavier tail: ${issues}`,
  );
});

test('a paid tail fails when the free seat is only in a different route', () => {
  const config = configWithWallSplit();
  config.experimental.model_routes['mixed-pool'] = {
    targets: ['paid/free-model#low'],
    weights: { 'paid/free-model': 0.1 },
  };
  const issues = inspectProviderWallRules(config);
  assert.ok(
    issues.some((issue) => issue.includes('"free-model"') && issue.includes('mixed-pool')),
    `expected a split invariant issue for the cross-route tail: ${issues}`,
  );
});

test('a paid tail fails when either seat lacks an explicit weight', () => {
  const config = configWithWallSplit();
  config.experimental.model_routes['mixed-pool'] = {
    targets: ['paid-free/free-model', 'paid/free-model#low'],
  };
  const issues = inspectProviderWallRules(config);
  assert.ok(
    issues.some((issue) => issue.includes('"free-model"') && issue.includes('mixed-pool')),
    `expected a split invariant issue for the unweighted tail: ${issues}`,
  );
});
