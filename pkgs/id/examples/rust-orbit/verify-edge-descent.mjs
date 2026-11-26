// Usage: node verify-edge-descent.mjs <built.wasm>
// Oracle: orbit-arcade/edge-descent in pkgs/id/book/research/evidence/app-vectors.json
// (revision b9d8177). Inputs and after-states are literal copies of the vector;
// nothing is derived from the guest. Exercises the actual guest wire interface,
// including reopening a snapshot in a fresh instance after the first tick.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

if (process.argv.length !== 3) {
  throw new Error('Usage: node verify-edge-descent.mjs <built.wasm>');
}

const initial = {
  tick: 3, cannon: [2, 3], targets: [{ id: 'T', at: [3, 0] }], formation_direction: 'E',
  shots: [], pending: [], score: 0, phase: 'active',
};
const queued = {
  tick: 3, cannon: [2, 3], targets: [{ id: 'T', at: [3, 0] }], formation_direction: 'E',
  shots: [], pending: [{ id: 'q4', for_tick: 4, steer: 1, fire: true }], score: 0, phase: 'active',
};
const afterT4 = {
  tick: 4, cannon: [3, 3], targets: [{ id: 'T', at: [3, 1] }], formation_direction: 'W',
  shots: [{ id: 'q4', at: [3, 2] }], pending: [], score: 0, phase: 'active',
};
const afterT5 = {
  tick: 5, cannon: [3, 3], targets: [], formation_direction: 'W',
  shots: [], pending: [], score: 10, phase: 'wave-clear',
};
const steps = [
  [{ kind: 'QueueControls', id: 'q4', for_tick: 4, steer: 1, fire: true }, 'queued', queued],
  [{ kind: 'Redeliver', ref: 'q4' }, 'already-folded', queued],
  [{ kind: 'Tick', number: 4, event: 't4' }, 'steer-fire-and-descend', afterT4],
  [{ kind: 'RenderFrames', count: 120 }, 'presentation-only', afterT4],
  [{ kind: 'Tick', number: 5, event: 't5' }, 'hit-T-and-clear', afterT5],
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
// Manual ticks: the wave asks the host for no timer and no capability.
assert.deepEqual(world.wants(), {});
for (const [index, [command, outcome, after]] of steps.entries()) {
  world.send(1, JSON.stringify(command));
  assert.deepEqual(world.record(), withOutcome(after, outcome), JSON.stringify(command));
  if (index === 2) {
    const reopened = open(world.snapshot());
    assert.deepEqual(reopened.record(), world.record());
    assert.deepEqual(reopened.screen(), world.screen());
    assert.equal(reopened.snapshot(), world.snapshot());
    world = reopened;
  }
}
// Viewing never ticks, and host (participant 0) deliveries never drive the wave.
const settled = world.record();
for (let frame = 0; frame < 120; frame++) world.screen();
world.send(0, JSON.stringify({ kind: 'Tick', number: 6 }));
assert.deepEqual(world.record(), settled);
const rows = world.screen().rows.map((row) => row.trimEnd());
assert.ok(rows.includes('. . . A'), rows.join('\n'));
assert.ok(rows.includes('tick=5 score=10 heading=W'), rows.join('\n'));
console.log('Orbit Arcade Plaza ABI oracle passed: five literal edge-descent states, fresh-instance restore, presentation-only frames and host isolation.');
