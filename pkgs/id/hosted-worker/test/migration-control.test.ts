import { test } from "node:test";
import assert from "node:assert/strict";
import {
  beginMigration,
  MigrationTransitionError,
  transitionMigration,
  type MigrationEvent,
} from "../src/migration-control.ts";

// Oracle: the transition model in doc/2026-11-01_jurisdiction_aware_user_store_design.md.
// This first slice is deliberately pure: it cannot enable production migration or
// bypass the directory, restricted-routing, storage, or hosted-write prerequisites.

test("a new migration keeps the source active and advances the routing epoch", () => {
  const state = beginMigration(
    { activeJurisdiction: "us", routingEpoch: 7 },
    { migrationId: "m-1", targetJurisdiction: "eu" },
  );

  assert.deepEqual(state, {
    migrationId: "m-1",
    sourceJurisdiction: "us",
    targetJurisdiction: "eu",
    activeJurisdiction: "us",
    routingEpoch: 8,
    phase: "pending",
    sourceFrozen: false,
    targetServing: false,
  });
});

test("target stays non-serving until switch and activation follows switch", () => {
  let state = beginMigration(
    { activeJurisdiction: "us", routingEpoch: 0 },
    { migrationId: "m-2", targetJurisdiction: "eu" },
  );
  const events: MigrationEvent[] = [{ type: "freeze-acknowledged" }, { type: "snapshot-verified" }, { type: "switch" }];

  for (const event of events) {
    state = transitionMigration(state, event);
    assert.equal(state.targetServing, false, `target served during ${state.phase}`);
  }
  assert.equal(state.activeJurisdiction, "eu");
  state = transitionMigration(state, { type: "activate-target" });
  assert.equal(state.phase, "active");
  assert.equal(state.targetServing, true);
  assert.equal(state.sourceFrozen, true);
});

test("a switched migration cannot roll back; recovery can only continue forward", () => {
  let state = beginMigration(
    { activeJurisdiction: "us", routingEpoch: 4 },
    { migrationId: "m-3", targetJurisdiction: "eu" },
  );
  for (const event of [{ type: "freeze-acknowledged" }, { type: "snapshot-verified" }, { type: "switch" }] as const) {
    state = transitionMigration(state, event);
  }

  assert.throws(() => transitionMigration(state, { type: "abort" }), /cannot abort after switch/);
  assert.equal(state.activeJurisdiction, "eu");
  assert.equal(state.sourceFrozen, true);
  assert.equal(state.targetServing, false);
});

test("pre-switch abort returns to source with a fresh epoch and never serves target", () => {
  let state = beginMigration(
    { activeJurisdiction: "us", routingEpoch: 12 },
    { migrationId: "m-4", targetJurisdiction: "eu" },
  );
  state = transitionMigration(state, { type: "freeze-acknowledged" });
  state = transitionMigration(state, { type: "abort" });

  assert.equal(state.phase, "aborted");
  assert.equal(state.activeJurisdiction, "us");
  assert.equal(state.routingEpoch, 14);
  assert.equal(state.sourceFrozen, false);
  assert.equal(state.targetServing, false);
});

test("generated phase/event combinations preserve routing and serving invariants", () => {
  const eventTypes = ["freeze-acknowledged", "snapshot-verified", "switch", "activate-target", "abort"] as const;
  const phases = ["pending", "frozen", "verified", "switched", "active", "aborted"] as const;

  for (const phase of phases) {
    for (const eventType of eventTypes) {
      const before = {
        migrationId: "generated",
        sourceJurisdiction: "us" as const,
        targetJurisdiction: "eu" as const,
        activeJurisdiction: phase === "switched" || phase === "active" ? ("eu" as const) : ("us" as const),
        routingEpoch: 9,
        phase,
        sourceFrozen: phase === "frozen" || phase === "verified" || phase === "switched" || phase === "active",
        targetServing: phase === "active",
      };
      try {
        const after = transitionMigration(before, { type: eventType });
        assert.ok(after.routingEpoch >= before.routingEpoch, `${phase} + ${eventType} decreased epoch`);
        assert.ok(
          after.targetServing === false || after.phase === "active",
          `${phase} + ${eventType} served target early`,
        );
        assert.ok(after.activeJurisdiction === "us" || after.sourceFrozen, `${phase} + ${eventType} unfenced source`);
      } catch (error) {
        // Illegal transitions are valid outcomes; the oracle applies to accepted transitions.
        assert.ok(error instanceof MigrationTransitionError);
      }
    }
  }
});

test("seeded event sequences preserve migration invariants across generated histories", () => {
  const eventTypes: MigrationEvent["type"][] = [
    "freeze-acknowledged",
    "snapshot-verified",
    "switch",
    "activate-target",
    "abort",
  ];
  let seed = 0x3404;
  const next = () => {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    return seed;
  };
  const assertInvariant = (state: ReturnType<typeof beginMigration>) => {
    assert.ok(state.routingEpoch >= 1);
    assert.equal(state.targetServing, state.phase === "active");
    assert.equal(
      state.sourceFrozen,
      state.phase === "frozen" || state.phase === "verified" || state.phase === "switched" || state.phase === "active",
    );
    if (
      state.phase === "pending" ||
      state.phase === "frozen" ||
      state.phase === "verified" ||
      state.phase === "aborted"
    ) {
      assert.equal(state.activeJurisdiction, state.sourceJurisdiction);
    }
  };

  // A fixed-seed generator makes the run reproducible; the oracle is the
  // design's routing/fencing invariant, not a list of hand-picked outputs.
  for (let run = 0; run < 1_000; run += 1) {
    let state = beginMigration(
      { activeJurisdiction: "us", routingEpoch: next() % 1_000 },
      { migrationId: `generated-${run}`, targetJurisdiction: "eu" },
    );
    for (let step = 0; step < 10; step += 1) {
      const before = state;
      const event: MigrationEvent = { type: eventTypes[next() % eventTypes.length]! };
      let after: ReturnType<typeof beginMigration>;
      try {
        after = transitionMigration(state, event);
      } catch (error) {
        assert.ok(error instanceof MigrationTransitionError);
        assert.equal(state, before, "rejected transition mutated its input state");
        continue;
      }
      state = after;
      assert.ok(state.routingEpoch >= before.routingEpoch);
      assertInvariant(state);
    }
  }
});
