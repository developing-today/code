// Isolate-side runner for the #2271 comparison harness.
//
// Drives the checked-in example guests through PlazaGuest (the Worker
// isolate host) over the exact scripted sequences the native reference
// runner uses (pkgs/id/tests/worker_poc_reference.rs), and emits one JSON
// object per step: {world, step, event, view, records, snapshot_hex}.
// Step 0 is the freshly instantiated state (event null); step N is the
// state after the Nth scripted event.
//
// Each sequence is run twice: uninterrupted, and with a forced cold start
// mid-sequence (fresh PlazaGuest restored from the mid-sequence snapshot).
// Both runs must produce identical lines — the cold start is part of the
// #2271 pass condition. `runIsolateSequences()` returns the uninterrupted
// lines for comparison against the native reference.
import { readFileSync } from "node:fs";
import { PlazaGuest } from "../src/guest.ts";

const enc = new TextEncoder();
const dec = new TextDecoder();
const hex = (b) => [...b].map((x) => x.toString(16).padStart(2, "0")).join("");

const SEQUENCES = [
  {
    world: "roc-counter",
    wasm: "../../examples/roc-counter/counter.wasm",
    events: ["inc", "inc", "inc"],
    coldStartAfter: 2,
  },
  {
    world: "tic-tac-toe",
    wasm: "../../apps/tic-tac-toe/program/tic-tac-toe.wasm",
    // X:0, O:4, X:1, O retries occupied 4 (ignored), 9 not a move
    // (ignored), O:8, X:2 wins, 3 after the win (ignored).
    events: ["0", "4", "1", "4", "9", "8", "2", "3"],
    coldStartAfter: 5,
  },
];

function stepLine(world, step, event, guest) {
  const snapshot = guest.snapshot();
  const records = guest.records();
  return {
    world,
    step,
    event,
    view: dec.decode(guest.view()),
    records: records ? dec.decode(records) : null,
    snapshot_hex: snapshot ? hex(snapshot) : null,
  };
}

async function runSequence({ world, wasm, events, coldStartAfter }, coldStart) {
  const bytes = readFileSync(new URL(wasm, import.meta.url));
  let guest = await PlazaGuest.instantiate(bytes, 0);
  const lines = [stepLine(world, 0, null, guest)];
  for (let i = 0; i < events.length; i++) {
    const step = i + 1;
    if (coldStart && step - 1 === coldStartAfter) {
      const snapshot = guest.snapshot();
      guest = await PlazaGuest.instantiate(bytes, 0);
      guest.restore(snapshot);
    }
    guest.update(1, enc.encode(events[i]));
    lines.push(stepLine(world, step, events[i], guest));
  }
  return lines;
}

export async function runIsolateSequences() {
  const all = [];
  for (const seq of SEQUENCES) {
    const uninterrupted = await runSequence(seq, false);
    const coldStarted = await runSequence(seq, true);
    const a = uninterrupted.map((l) => JSON.stringify(l));
    const b = coldStarted.map((l) => JSON.stringify(l));
    if (a.join("\n") !== b.join("\n")) {
      throw new Error(
        `isolate host diverged across a forced cold start after step ${seq.coldStartAfter} in ${seq.world}`,
      );
    }
    all.push(...uninterrupted);
  }
  return all;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  for (const line of await runIsolateSequences()) console.log(JSON.stringify(line));
}
