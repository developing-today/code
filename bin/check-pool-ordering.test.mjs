import { test } from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { inspectPoolOrdering, parseArgs } from './check-pool-ordering.mjs';

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
          'muse-wonder-1.0-contributor': { variants: { low: {}, medium: {}, high: {} } },
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

test('parseArgs rejects undeclared single-dash flags before reading any config', () => {
  // `-j` is not a declared flag: it must be a usage error, never the
  // config-path positional (which would audit whatever file it names).
  assert.throws(() => parseArgs(['-j']), /unrecognised argument: -j/);
  assert.throws(() => parseArgs(['-h']), /unrecognised argument: -h/);
  assert.throws(() => parseArgs(['-']), /unrecognised argument: -/);
  assert.throws(() => parseArgs(['--bogus']), /unrecognised argument: --bogus/);
  assert.throws(() => parseArgs(['--json', '--json']), /--json was given more than once/);
  assert.throws(() => parseArgs(['a.jsonc', 'b.jsonc']), /unexpected positional argument: b\.jsonc/);
  assert.equal(parseArgs(['--json']).json, true);
  assert.equal(parseArgs(['some/config.jsonc']).configPath, path.resolve('some/config.jsonc'));
});

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

test('transitive fallback boundaries split nested fallback passes', () => {
  // scout -> background -> workhorse: background's own chain ends in the
  // OpenAI-backed luna-plan, and workhorse's chain starts with the
  // non-OpenAI haiku-plan. Workhorse is background's declared
  // exhaustion-only fallback, so the two chains are separate passes and
  // there is no violation — grouping by the tier target alone collapsed
  // them into one ordered sequence and reported a false violation.
  const config = configWith({
    tiers: {
      scout: {
        targets: ['opencode-route/background', 'opencode-route/workhorse'],
        selection: 'ordered',
      },
      workhorse: {
        targets: ['opencode-route/haiku-plan'],
        selection: 'ordered',
      },
    },
    pools: {
      background: {
        targets: ['opencode-route/luna-plan', 'opencode-route/workhorse'],
        selection: 'ordered',
      },
    },
  });
  const result = inspectPoolOrdering(config);
  assert.deepEqual(result.issues, []);
  assert.equal(result.tiers.scout.passes.length, 2);
  assert.deepEqual(result.tiers.scout.passes[0], {
    label: 'fallback via background',
    pools: ['luna-plan'],
    ordered: { 'luna-plan': true },
    openai_backed: { 'luna-plan': true },
    reserve: 'pass',
  });
  assert.deepEqual(result.tiers.scout.passes[1], {
    label: 'fallback via background -> workhorse',
    pools: ['haiku-plan'],
    ordered: { 'haiku-plan': true },
    openai_backed: { 'haiku-plan': false },
    reserve: 'n/a',
  });
});

test('the same pools in one chain without a nested fallback boundary are a violation (control)', () => {
  // Background's chain runs luna-plan then haiku-plan with no fallback
  // edge between them: one ordered pass, so the OpenAI-backed pool
  // before a non-OpenAI pool is a real violation.
  const config = configWith({
    tiers: {
      scout: {
        targets: ['opencode-route/background', 'opencode-route/workhorse'],
        selection: 'ordered',
      },
      workhorse: {
        targets: ['opencode-route/haiku-plan'],
        selection: 'ordered',
      },
    },
    pools: {
      background: {
        targets: ['opencode-route/luna-plan', 'opencode-route/haiku-plan'],
        selection: 'ordered',
      },
    },
  });
  const result = inspectPoolOrdering(config);
  assert.equal(result.issues.length, 1);
  assert.match(result.issues[0], /scout fallback via background/);
  assert.match(result.issues[0], /luna-plan/);
  assert.match(result.issues[0], /haiku-plan/);
  assert.equal(result.tiers.scout.reserve, 'violation');
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

test('distinct direct leaves from one provider stay distinct pools', () => {
  // Spark (direct _muse) -> sol-plan (OpenAI-backed) -> Wonder (direct
  // _muse): the trailing non-OpenAI Wonder seat must not collapse into
  // the earlier Spark seat and disappear from the check.
  const config = configWith({
    tiers: {
      summit: {
        targets: [
          '_muse/muse-spark-1.3-contributor#medium',
          'opencode-route/sol-plan#high',
          '_muse/muse-wonder-1.0-contributor#medium',
        ],
        selection: 'ordered',
      },
    },
  });
  const result = inspectPoolOrdering(config);
  assert.deepEqual(result.tiers.summit.passes[0].pools, [
    'direct:_muse/muse-spark-1.3-contributor',
    'sol-plan',
    'direct:_muse/muse-wonder-1.0-contributor',
  ]);
  assert.equal(result.issues.length, 1);
  assert.match(result.issues[0], /summit/);
  assert.match(result.issues[0], /sol-plan/);
  assert.match(result.issues[0], /muse-wonder-1\.0-contributor/);
  assert.equal(result.tiers.summit.reserve, 'violation');
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

// --- Generated (property / model-based) coverage for the reserve-order
// invariant (AGENTS.md testing doctrine). A seeded deterministic
// generator builds route configurations, and an independent model —
// computed from the generated spec, not from the checker's grouping
// code — states the expected passes and reserve verdicts. Oracle: in
// every ordered pass an OpenAI-backed pool must come last; passes split
// at every class-fallback edge, transitively; unordered passes carry no
// verdict. The generator covers pool permutations, selection modes,
// direct leaves (including same-provider distinct models), and
// transitive fallback boundaries (scout -> background -> workhorse).

// Seeded deterministic PRNG (mulberry32) — no new dependency.
function mulberry32(seed) {
  let a = seed >>> 0;
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// Direct leaves the generator may seat straight in a tier chain, with
// the pool id the checker must report for them.
const GEN_DIRECT = [
  { ref: '_muse/muse-spark-1.3-contributor', pool: 'direct:_muse/muse-spark-1.3-contributor', openai: false },
  { ref: '_muse/muse-wonder-1.0-contributor', pool: 'direct:_muse/muse-wonder-1.0-contributor', openai: false },
  { ref: '_claude/claude-haiku-5-5', pool: 'direct:_claude/claude-haiku-5-5', openai: false },
  { ref: '_claude/claude-fable-5-2', pool: 'direct:_claude/claude-fable-5-2', openai: false },
  { ref: 'openai/gpt-6.1-sol', pool: 'direct:openai/gpt-6.1-sol', openai: true },
];

// Leaves a generated pool route may hold. A pool is OpenAI-backed when
// any of its leaves is.
const GEN_POOL_LEAVES = [
  { ref: 'openai-plan/gpt-6.1-sol', openai: true },
  { ref: 'openai-plan/gpt-6-luna', openai: true },
  { ref: '_muse/muse-spark-1.3-contributor', openai: false },
  { ref: '_claude/claude-haiku-5-5', openai: false },
];

const GEN_SELECTIONS = ['ordered', 'ordered', 'ordered', 'weighted', 'round-robin'];

function genPick(rng, array) {
  return array[Math.floor(rng() * array.length)];
}

function genSample(rng, array, count) {
  const copy = [...array];
  for (let i = copy.length - 1; i > 0; i -= 1) {
    const j = Math.floor(rng() * (i + 1));
    [copy[i], copy[j]] = [copy[j], copy[i]];
  }
  return copy.slice(0, count);
}

// Take up to `max` items from `remaining` (without replacement, so a
// route is expanded once per tier, matching the resolver's seen set).
function takeChain(rng, remaining, max) {
  const count = Math.floor(rng() * (max + 1));
  const items = genSample(rng, remaining, Math.min(count, remaining.length));
  for (const item of items) remaining.splice(remaining.indexOf(item), 1);
  return items;
}

// Same, restricted to pool items: nested class-route chains hold pools
// only (a direct leaf nested in a class route pools under that route's
// name by the checker's existing chain-route semantics, which this
// generator does not model).
function takePoolChain(rng, remaining, max) {
  const poolOnly = remaining.filter((item) => item.kind === 'pool');
  const count = Math.floor(rng() * (max + 1));
  const items = genSample(rng, poolOnly, Math.min(count, poolOnly.length));
  for (const item of items) remaining.splice(remaining.indexOf(item), 1);
  return items;
}

// The independent model: the expected pass for one chain of items under
// the given governing-route selections, in the checker's pass shape.
function modelPass(label, items, ancestorSelections) {
  const pools = [];
  const ordered = {};
  const backed = {};
  const passOrdered = ancestorSelections.every((selection) => selection === 'ordered');
  for (const item of items) {
    const pool = item.kind === 'pool' ? item.name : item.pool;
    if (!pools.includes(pool)) pools.push(pool);
    backed[pool] = item.openai;
    ordered[pool] = passOrdered;
  }
  const orderedSeq = passOrdered ? pools : [];
  const firstOpenAi = orderedSeq.findIndex((pool) => backed[pool]);
  let reserve = 'n/a';
  if (firstOpenAi >= 0) {
    reserve = orderedSeq.slice(firstOpenAi + 1).some((pool) => !backed[pool])
      ? 'violation'
      : 'pass';
  }
  return { label, pools, ordered, openai_backed: backed, reserve };
}

function modelTierReserve(passes) {
  if (passes.some((pass) => pass.reserve === 'violation')) return 'violation';
  if (passes.some((pass) => pass.reserve === 'pass')) return 'pass';
  return 'n/a';
}

function generatedCase(seed) {
  const rng = mulberry32(seed);
  const pools = {};
  const poolItems = [];
  for (let k = 0; k < 4; k += 1) {
    const name = `gen-pool-${k}`;
    const leaves = genSample(rng, GEN_POOL_LEAVES, 1 + Math.floor(rng() * 2));
    pools[name] = { targets: leaves.map((leaf) => leaf.ref), selection: 'ordered' };
    poolItems.push({ kind: 'pool', name, openai: leaves.some((leaf) => leaf.openai) });
  }
  const directItems = GEN_DIRECT.map((leaf) => ({ kind: 'direct', ...leaf }));
  const refOf = (item) => (item.kind === 'pool' ? `opencode-route/${item.name}` : item.ref);

  // scout: main chain + background/workhorse fallback tails. The
  // workhorse route doubles as scout's fallback target and as the
  // workhorse tier, as in the canonical config.
  const scoutSelection = genPick(rng, GEN_SELECTIONS);
  const backgroundSelection = genPick(rng, GEN_SELECTIONS);
  const workhorseSelection = genPick(rng, GEN_SELECTIONS);
  const scoutItems = [...poolItems, ...directItems];
  const main = takeChain(rng, scoutItems, 4);
  const bg = takePoolChain(rng, scoutItems, 3);
  const bgHasWorkhorseTail = rng() < 0.5;
  const wh = takePoolChain(rng, scoutItems, 3);

  const summitSelection = genPick(rng, GEN_SELECTIONS);
  const summit = takeChain(rng, [...poolItems, ...directItems], 4);
  const specialistSelection = genPick(rng, GEN_SELECTIONS);
  const specialist = takeChain(rng, [...poolItems, ...directItems], 4);

  const config = configWith({
    tiers: {
      scout: {
        targets: [...main.map(refOf), 'opencode-route/background', 'opencode-route/workhorse'],
        selection: scoutSelection,
      },
      workhorse: { targets: wh.map(refOf), selection: workhorseSelection },
      summit: { targets: summit.map(refOf), selection: summitSelection },
      specialist: { targets: specialist.map(refOf), selection: specialistSelection },
    },
    pools: {
      ...pools,
      background: {
        targets: [...bg.map(refOf), ...(bgHasWorkhorseTail ? ['opencode-route/workhorse'] : [])],
        selection: backgroundSelection,
      },
    },
  });

  const expected = {
    scout: [
      ...(main.length ? [modelPass('main chain', main, [scoutSelection])] : []),
      ...(bg.length
        ? [modelPass('fallback via background', bg, [scoutSelection, backgroundSelection])]
        : []),
      ...(wh.length
        ? [
            modelPass(
              bgHasWorkhorseTail ? 'fallback via background -> workhorse' : 'fallback via workhorse',
              wh,
              bgHasWorkhorseTail
                ? [scoutSelection, backgroundSelection, workhorseSelection]
                : [scoutSelection, workhorseSelection],
            ),
          ]
        : []),
    ],
    workhorse: wh.length ? [modelPass('main chain', wh, [workhorseSelection])] : [],
    summit: summit.length ? [modelPass('main chain', summit, [summitSelection])] : [],
    specialist: specialist.length
      ? [modelPass('main chain', specialist, [specialistSelection])]
      : [],
  };
  return { config, expected };
}

test('generated configs match an independent reserve-order model', () => {
  // 32 fixed seeds: fresh chains, pool mixes, direct leaves and
  // fallback shapes per seed. The checker must reproduce the model's
  // passes and reserve verdicts exactly, and report exactly the
  // violations the model predicts.
  for (let seed = 1; seed <= 32; seed += 1) {
    const { config, expected } = generatedCase(seed);
    const result = inspectPoolOrdering(config);
    for (const tier of ['scout', 'workhorse', 'specialist', 'summit']) {
      assert.deepEqual(result.tiers[tier].passes, expected[tier], `seed ${seed} tier ${tier}`);
      assert.equal(
        result.tiers[tier].reserve,
        modelTierReserve(expected[tier]),
        `seed ${seed} tier ${tier} reserve`,
      );
    }
    const fragments = [];
    for (const tier of ['scout', 'workhorse', 'specialist', 'summit']) {
      for (const pass of expected[tier]) {
        const orderedSeq = pass.pools.filter((pool) => pass.ordered[pool]);
        const first = orderedSeq.findIndex((pool) => pass.openai_backed[pool]);
        if (first < 0) continue;
        const later = orderedSeq.slice(first + 1).find((pool) => !pass.openai_backed[pool]);
        if (later) {
          fragments.push(
            `${tier} ${pass.label}: OpenAI-backed pool "${orderedSeq[first]}" ` +
              `is tried before non-OpenAI pool "${later}"`,
          );
        }
      }
    }
    assert.equal(
      result.issues.length,
      fragments.length,
      `seed ${seed}: expected ${fragments.length} issue(s), got ${JSON.stringify(result.issues)}`,
    );
    for (const fragment of fragments) {
      assert.ok(
        result.issues.some((issue) => issue.includes(fragment)),
        `seed ${seed}: missing issue ${fragment}`,
      );
    }
  }
});
