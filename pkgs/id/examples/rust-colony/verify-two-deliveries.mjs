// Usage: node verify-two-deliveries.mjs <built.wasm>
// Oracle: colony/two-deliveries in pkgs/id/book/research/evidence/app-vectors.json
// (revision b9d8177). Inputs and after-states are literal copies of the vector;
// nothing is derived from the guest. Exercises the actual guest wire interface,
// including reopening a snapshot in a fresh instance after the first deposit.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

if (process.argv.length !== 3) {
  throw new Error('Usage: node verify-two-deliveries.mjs <built.wasm>');
}

const initial = { worker: [1, 1], ore_cells: [[1, 0], [2, 1]], pack_ore: 0, stored_ore: 0, phase: 'working' };
const steps = [
  [{ kind: 'Mine', direction: 'E', event: 'm1' }, 'extracted',
    { worker: [1, 1], ore_cells: [[1, 0]], pack_ore: 1, stored_ore: 0, phase: 'working' }],
  [{ kind: 'Mine', direction: 'N', event: 'm2' }, 'pack-full',
    { worker: [1, 1], ore_cells: [[1, 0]], pack_ore: 1, stored_ore: 0, phase: 'working' }],
  [{ kind: 'Walk', direction: 'W', event: 'w1' }, 'moved',
    { worker: [0, 1], ore_cells: [[1, 0]], pack_ore: 1, stored_ore: 0, phase: 'working' }],
  [{ kind: 'Deposit', event: 'd1' }, 'stored',
    { worker: [0, 1], ore_cells: [[1, 0]], pack_ore: 0, stored_ore: 1, phase: 'working' }],
  [{ kind: 'Walk', direction: 'E', event: 'w2' }, 'moved',
    { worker: [1, 1], ore_cells: [[1, 0]], pack_ore: 0, stored_ore: 1, phase: 'working' }],
  [{ kind: 'Mine', direction: 'N', event: 'm3' }, 'extracted',
    { worker: [1, 1], ore_cells: [], pack_ore: 1, stored_ore: 1, phase: 'working' }],
  [{ kind: 'Walk', direction: 'W', event: 'w3' }, 'moved',
    { worker: [0, 1], ore_cells: [], pack_ore: 1, stored_ore: 1, phase: 'working' }],
  [{ kind: 'Deposit', event: 'd2' }, 'supply-goal-met',
    { worker: [0, 1], ore_cells: [], pack_ore: 0, stored_ore: 2, phase: 'supplied' }],
];

const module = await WebAssembly.compile(await readFile(process.argv[2]));
assert.deepEqual(WebAssembly.Module.imports(module), []);
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: true });

function open(snapshot) {
  const e = new WebAssembly.Instance(module).exports;
  assert.equal(e.plaza_abi_version(), 1);
  function input(value, invoke) {
    const bytes = encoder.encode(value);
    const ptr = e.plaza_alloc(bytes.length);
    try {
      new Uint8Array(e.memory.buffer, ptr, bytes.length).set(bytes);
      return invoke(ptr, bytes.length);
    } finally {
      e.plaza_free(ptr, bytes.length);
    }
  }
  let model = snapshot === undefined ? e.plaza_init(0n) : input(snapshot, e.plaza_restore);
  function output(callback) {
    const ptr = callback(model);
    return decoder.decode(new Uint8Array(e.memory.buffer, ptr, e.plaza_out_len()));
  }
  return {
    send(participant, value) {
      model = input(value, (ptr, length) => e.plaza_update(model, participant, ptr, length));
    },
    record() { return JSON.parse(output(e.plaza_records)); },
    screen() { return JSON.parse(output(e.plaza_view)); },
    wants() { return JSON.parse(output(e.plaza_wants)); },
    snapshot() { return output(e.plaza_snapshot); },
  };
}

const withOutcome = (state, outcome) => ({ ...state, outcome });
let world = open();
assert.deepEqual(world.record(), withOutcome(initial, ''));
// The colony asks the host for no timer and no capability.
assert.deepEqual(world.wants(), {});
for (const [index, [command, outcome, after]] of steps.entries()) {
  world.send(1, JSON.stringify(command));
  assert.deepEqual(world.record(), withOutcome(after, outcome), JSON.stringify(command));
  if (index === 3) {
    const reopened = open(world.snapshot());
    assert.deepEqual(reopened.record(), world.record());
    assert.deepEqual(reopened.screen(), world.screen());
    assert.equal(reopened.snapshot(), world.snapshot());
    world = reopened;
  }
}
// Viewing never changes the domain, host (participant 0) deliveries never
// drive the colony, and a replayed event folds once.
const settled = world.record();
for (let frame = 0; frame < 120; frame++) world.screen();
world.send(0, JSON.stringify(steps[0][0]));
assert.deepEqual(world.record(), settled);
world.send(1, JSON.stringify(steps[7][0]));
assert.deepEqual(world.record(), withOutcome(steps[7][2], 'already-folded'));
const rows = world.screen().rows.map((row) => row.trimEnd());
assert.ok(rows.includes('@ . . .'), rows.join('\n'));
assert.ok(rows.includes('pack=0/1 stored=2/2'), rows.join('\n'));
console.log('Colony Plaza ABI oracle passed: eight literal two-deliveries states, fresh-instance restore, presentation-only views, host isolation and replay folding.');
