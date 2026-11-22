import { test } from 'node:test';
import assert from 'node:assert/strict';
import { inspectPoolOrdering } from './check-pool-ordering.mjs';

// Oracle: the tier's effective pool order, read off inspectRouteRules'
// resolved eligible leaves. In every ordered chain an OpenAI-backed pool
// must come last (the fleet's deepest reserve: the design chain "spends
// Antigravity, the Go contributor bucket, and Meta before it touches
// OpenAI at all", .opencode/opencode.jsonc). In weighted/round-robin
// routes list position is not burn order, so mixing there is a note
// (unknown), never a finding.

function configWith({ tiers = {}, pools = {}, providerRoutes = {} } = {}) {
  const basePools = {
    'sol-plan': {
      targets: ['openai-plan/gpt-6.1-sol', 'opendesign/gpt-6.1-sol'],
      selection: 'round-robin',
    },
    'luna-plan': {
      targets: ['openai-plan/gpt-6-luna', 'opendesign/gpt-6-luna'],
      selection: 'round-robin',
    },
    'spark-plan': {
      targets: ['_muse/muse-spark-1.3-contributor'],
      selection: 'ordered',
    },
    'fable-plan': {
      targets: ['_claude/claude-fable-5-2'],
      selection: 'ordered',
    },
    'haiku-plan': {
      targets: ['_claude/claude-haiku-5-5'],
      selection: 'ordered',
    },
    'deepseek-plan': {
      targets: ['deepseek/deepseek-model'],
      selection: 'ordered',
    },
    ...pools,
  };
  const baseTiers = {
    scout: { targets: ['opencode-route/spark-plan'], selection: 'ordered' },
    workhorse: { targets: ['opencode-route/spark-plan'], selection: 'ordered' },
    specialist: {
      targets: [
        'opencode-route/fable-plan#low',
        'opencode-route/haiku-plan#high',
        'opencode-route/spark-plan#medium',
        'opencode-route/sol-plan#medium',
      ],
      selection: 'ordered',
    },
    summit: {
      targets: ['opencode-route/spark-plan', 'opencode-route/sol-plan#high'],
      selection: 'ordered',
    },
    ...tiers,
  };
  return {
    disabled_providers: [],
    provider: {
      openai: {
        models: {
          'gpt-6.1-sol': { variants: { low: {}, medium: {}, high: {} } },
          'gpt-6-luna': { variants: { low: {}, medium: {}, high: {} } },
        },
      },
      '_openai.alt': {
        instanceOf: 'openai',
        models: {
          'gpt-6.1-sol': { variants: { low: {}, medium: {}, high: {} } },
          'gpt-6-luna': { variants: { low: {}, medium: {}, high: {} } },
        },
      },
      '_openai.main': {
        instanceOf: 'openai',
        models: {
          'gpt-6.1-sol': { variants: { low: {}, medium: {}, high: {} } },
          'gpt-6-luna': { variants: { low: {}, medium: {}, high: {} } },
        },
      },
      opendesign: {
        models: {
          'gpt-6.1-sol': { variants: { low: {}, medium: {}, high: {} } },
          'gpt-6-luna': { variants: { low: {}, medium: {}, high: {} } },
        },
      },
      _muse: {
        models: {
          'muse-spark-1.3-contributor': { variants: { low: {}, medium: {}, high: {} } },
        },
      },
      _claude: {
        models: {
          'claude-fable-5-2': { variants: { low: {}, medium: {}, high: {} } },
          'claude-haiku-5-5': { variants: { low: {}, medium: {}, high: {} } },
        },
      },
      deepseek: {
        models: { 'deepseek-model': { variants: { low: {}, high: {} } } },
      },
    },
    experimental: {
      provider_routes: {
        'openai-plan': {
          providers: ['_openai.alt', '_openai.main'],
          selection: 'round-robin',
        },
        ...providerRoutes,
      },
      model_routes: { ...basePools, ...baseTiers },
    },
  };
}

test('ordered chain with the OpenAI-backed pool last passes and reports pool order', () => {
  const result = inspectPoolOrdering(configWith());
  assert.deepEqual(result.issues, []);
  assert.equal(result.tiers.specialist.reserve, 'pass');
  assert.deepEqual(result.tiers.specialist.passes[0].pools, [
    'fable-plan',
    'haiku-plan',
    'spark-plan',
    'sol-plan',
  ]);
  assert.deepEqual(result.tiers.specialist.passes[0].openai_backed, {
    'fable-plan': false,
    'haiku-plan': false,
    'spark-plan': false,
    'sol-plan': true,
  });
});

test('ordered chain with the OpenAI-backed pool before a non-OpenAI pool is a finding', () => {
  const config = configWith({
    tiers: {
      summit: {
        targets: ['opencode-route/sol-plan#high', 'opencode-route/spark-plan#medium'],
        selection: 'ordered',
      },
    },
  });
  const result = inspectPoolOrdering(config);
  assert.equal(result.issues.length, 1);
  assert.match(result.issues[0], /summit/);
  assert.match(result.issues[0], /sol-plan/);
  assert.match(result.issues[0], /spark-plan/);
  assert.equal(result.tiers.summit.reserve, 'violation');
});

test('fallback passes are checked separately from the main chain', () => {
  // scout's declared fallbacks are background, workhorse: the same pools
  // that would violate in one chain pass when the fallback boundary
  // separates them.
  const config = configWith({
    tiers: {
      scout: {
        targets: [
          'opencode-route/spark-plan',
          'opencode-route/sol-plan#medium',
          'opencode-route/background',
          'opencode-route/workhorse',
        ],
        selection: 'ordered',
      },
      // The workhorse route doubles as scout's fallback target: one
      // route serves both roles, as in the canonical config.
      workhorse: {
        targets: ['opencode-route/haiku-plan'],
        selection: 'ordered',
      },
    },
    pools: {
      background: {
        targets: ['opencode-route/fable-plan'],
        selection: 'ordered',
      },
    },
  });
  const result = inspectPoolOrdering(config);
  assert.deepEqual(result.issues, []);
  assert.equal(result.tiers.scout.passes.length, 3);
  assert.deepEqual(result.tiers.scout.passes[0].pools, ['spark-plan', 'sol-plan']);
  assert.deepEqual(result.tiers.scout.passes[1].pools, ['fable-plan']);
  assert.deepEqual(result.tiers.scout.passes[2].pools, ['haiku-plan']);
});

test('the same pools without a fallback boundary are a violation (control)', () => {
  const config = configWith({
    tiers: {
      scout: {
        targets: [
          'opencode-route/spark-plan',
          'opencode-route/sol-plan#medium',
          'opencode-route/fable-plan',
        ],
        selection: 'ordered',
      },
    },
  });
  const result = inspectPoolOrdering(config);
  assert.equal(result.issues.length, 1);
  assert.match(result.issues[0], /scout/);
});

test('weighted mixing is a note, not a finding: position is not burn order there', () => {
  const config = configWith({
    tiers: {
      workhorse: {
        targets: ['opencode-route/workhorse-weighted'],
        selection: 'ordered',
      },
    },
    pools: {
      'workhorse-weighted': {
        targets: [
          'opencode-route/luna-plan#medium',
          '_muse/muse-spark-1.3-contributor#medium',
        ],
        selection: 'weighted',
      },
    },
  });
  const result = inspectPoolOrdering(config);
  assert.deepEqual(result.issues, []);
  assert.equal(result.notes.length, 1);
  assert.match(result.notes[0], /luna-plan/);
  assert.match(result.notes[0], /workhorse-weighted/);
});

test('funds leaves do not count toward pool order', () => {
  // deepseek-plan's only leaf is metered (funds): it never serves as
  // pool quota, so an OpenAI-backed pool before it is not a violation.
  const config = configWith({
    tiers: {
      summit: {
        targets: ['opencode-route/sol-plan#high', 'opencode-route/deepseek-plan#high'],
        selection: 'ordered',
      },
    },
  });
  const result = inspectPoolOrdering(config);
  assert.deepEqual(result.issues, []);
  assert.deepEqual(result.tiers.summit.passes[0].pools, ['sol-plan']);
});
