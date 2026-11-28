// Usage: node verify-glider.mjs <built.wasm>
// Oracle: literal row-major live-cell sets for the four B3/S23 generations.
// Exercise the actual guest wire interface, including reopening in a fresh instance.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

if (process.argv.length !== 3) {
  throw new Error('Usage: node verify-glider.mjs <built.wasm>');
}

const expected = [
  [[1, 0], [2, 1], [0, 2], [1, 2], [2, 2]],
  [[0, 1], [2, 1], [1, 2], [2, 2], [1, 3]],
  [[2, 1], [0, 2], [2, 2], [1, 3], [2, 3]],
  [[1, 1], [2, 2], [3, 2], [1, 3], [2, 3]],
  [[2, 1], [3, 2], [1, 3], [2, 3], [3, 3]],
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
    step(command) {
      model = input(command, (ptr, length) => e.plaza_update(model, 1, ptr, length));
    },
    record() { return JSON.parse(output(e.plaza_records)); },
    screen() { return JSON.parse(output(e.plaza_view)); },
    layout() { return JSON.parse(output(m => e.plaza_layout(m, 24n, 7n))); },
    wants() { return JSON.parse(output(e.plaza_wants)); },
    snapshot() { return output(e.plaza_snapshot); },
  };
}

let world = open();
for (let generation = 0; generation < expected.length; generation++) {
  if (generation > 0) world.step('step');
  if (generation === 2) world = open(world.snapshot());
  const projection = { generation, live: expected[generation] };
  assert.deepEqual(world.record(), projection);
  assert.deepEqual(world.wants(), {});
  const view = world.screen();
  assert.equal(view.kind, 'screen');
  assert.deepEqual(world.layout(), view);
  for (let y = 0; y < 5; y++) {
    const row = Array.from({ length: 5 }, (_, x) =>
      projection.live.some(([cx, cy]) => cx === x && cy === y) ? '#' : '.').join('');
    assert.equal(view.rows[y + 1].trimEnd(), row);
  }
  world.step('unknown');
  assert.deepEqual(world.record(), projection);
}
world.step('reset');
world.step('toggle 4 4');
assert.deepEqual(world.record(), { generation: 0, live: [...expected[0], [4, 4]] });
world.step('toggle 4 4');
assert.deepEqual(world.record(), { generation: 0, live: expected[0] });
console.log('Cells Plaza ABI oracle passed: four exact glider generations, fresh-instance restore, screen parity and manual input.');
