// Regression for #4256: a failure after guest.update() but before the journal
// insert must discard the guest, so the next /step starts from the journal.
// Run with `npm run post-update-failure`.
import assert from "node:assert/strict";
import { register } from "node:module";

// Register before importing worker.ts, whose Wasm import is normally
// transformed by Wrangler's CompiledWasm rule.
register("./compiled-wasm-loader.mjs", import.meta.url);
const [{ WorldObject }, { PlazaGuest }] = await Promise.all([
  import("../src/worker.ts"),
  import("../src/guest.ts"),
]);

class MemoryJournal {
  rows = [];

  exec(query, ...params) {
    if (query.startsWith("CREATE TABLE")) return { toArray: () => [] };
    if (query.startsWith("SELECT step, snapshot FROM journal")) {
      const latest = [...this.rows].sort((a, b) => b.step - a.step).slice(0, 1);
      return { toArray: () => latest.map((row) => ({ step: row.step, snapshot: row.snapshot })) };
    }
    // #6373: /cold-start asks only for the latest journaled step, without the
    // snapshot. Answer from the journaled rows so the cold-start assertion
    // still exercises the real journal rather than a hard-coded value.
    if (query.startsWith("SELECT step FROM journal")) {
      const latest = [...this.rows].sort((a, b) => b.step - a.step).slice(0, 1);
      return { toArray: () => latest.map((row) => ({ step: row.step })) };
    }
    if (query.startsWith("INSERT INTO journal")) {
      const [step, participant, event, snapshot, view, records, snapshotHash] = params;
      this.rows.push({
        step,
        participant,
        event,
        snapshot: new Uint8Array(snapshot),
        view,
        records,
        snapshotHash,
      });
      return { toArray: () => [] };
    }
    throw new Error(`unexpected SQL: ${query}`);
  }
}

const journal = new MemoryJournal();
const world = new WorldObject({ storage: { sql: journal } });
const originalView = PlazaGuest.prototype.view;
let shouldThrow = true;
PlazaGuest.prototype.view = function () {
  const view = originalView.call(this);
  if (shouldThrow) {
    shouldThrow = false;
    throw new Error("one-shot post-update view failure");
  }
  return view;
};

async function post(path, body) {
  return world.fetch(new Request(`https://worker-poc.test${path}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  }));
}

async function get(path) {
  return world.fetch(new Request(`https://worker-poc.test${path}`));
}

try {
  await assert.rejects(
    post("/step", { participant: 1, event: "inc" }),
    /one-shot post-update view failure/,
  );
  assert.equal(journal.rows.length, 0, "failed step must not be journaled");

  const response = await post("/step", { participant: 1, event: "inc" });
  assert.equal(response.status, 200);
  const secondStep = await response.json();
  assert.equal(secondStep.step, 1, "second /step starts from the unchanged journal");
  assert.equal(JSON.parse(secondStep.records).count, 1, "only the second event is reflected");
  assert.equal(journal.rows.length, 1, "journal contains exactly the successful step");
  assert.equal(journal.rows[0].step, 1);
  assert.equal(journal.rows[0].event, "inc");
  assert.equal(JSON.parse(journal.rows[0].records).count, 1);

  const cold = await post("/cold-start");
  assert.equal(cold.status, 200);
  const restoredResponse = await get("/state");
  assert.equal(restoredResponse.status, 200);
  const restored = await restoredResponse.json();
  assert.equal(restored.step, 1, "cold start restores the only journaled step");
  assert.equal(JSON.parse(restored.records).count, 1, "cold start has no phantom event");
  assert.equal(restored.view, secondStep.view);
  assert.equal(restored.snapshotHash, secondStep.snapshotHash);

  console.log("post-update-failure: PASS");
} finally {
  PlazaGuest.prototype.view = originalView;
}
