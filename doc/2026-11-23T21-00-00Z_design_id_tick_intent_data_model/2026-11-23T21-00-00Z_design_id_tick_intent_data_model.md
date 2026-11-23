# Design: tick and intent data model

- **Date:** 2026-11-23
- **Status:** proposed design record; no implementation is included here.
- **Issue:** [#5968](https://github.com/developing-today/code/issues/5968), part of the worlds design program [#5955](https://github.com/developing-today/code/issues/5955).
- **Related:** [#5779](https://github.com/developing-today/code/issues/5779) (deterministic testing and the simulated clock), [#5967](https://github.com/developing-today/code/issues/5967) (per-user function mapping, see [its design](../2026-11-08T10-26-00Z_design_id_per_user_function_mapping/2026-11-08T10-26-00Z_design_id_per_user_function_mapping.md)), [#5964](https://github.com/developing-today/code/issues/5964) (hot-swap), [#5966](https://github.com/developing-today/code/issues/5966) (fuel and timeout).
- **Capability protocol extended:** `pkgs/id/examples/roc-world/RocCap.roc`.
- **Settled premises:** the gating decision recorded on #5968 on 2026-11-22: human input wins over function intent for the same entity and tick, except where the world marks that entity as delegated; the world owns one per-user visibility projection, and presentation and function channels apply channel-specific shaping rather than recomputing visibility.

## Request and scope

Specify the per-tick data flow between the world, clients and user functions: what a user function may see, how intents from human input and from functions reach the same entity in the same tick, and how tick inputs and intents replay deterministically.

Out of scope: the rebind and artifact loading (#5964), the fuel cost table and cutoff mechanism (#5966), the world per-tick budget invariants (#5962), and the per-user binding and switching model (#5967). Where this design depends on those, it names the seam.

## 0. Definitions

- **World:** the authoritative simulation. It is the only writer of positions, scores and outcomes.
- **View:** a per-user, per-tick projection of world state. It is data only and carries no authority.
- **Intent:** a per-entity, per-tick desired action for the next tick. It is advisory; the world resolves it.
- **Presentation:** client rendering input, derived from the same projection at a different fidelity (§5).

## 1. Tick sequence

Each tick `t` has a monotonic `tick_id: U64` and a fixed `dt_ms` from the world's declared cadence. The tick id is the only clock the world reads.

1. **Collect inputs.** Gather human client inputs received during the input window of tick `t-1`, and function intents submitted against the view of tick `t-1`. Anything arriving after the window closes is late (§3).
2. **Resolve precedence.** For each entity, pick exactly one effective intent for tick `t` using the blending rules (§4). An entity with no intent takes its fallback (§3).
3. **Step the world.** A pure function of `(state[t-1], effective_intents[t], tick_id)`. It updates positions, runs calculations and resolves interactions. No I/O, no wall-clock reads, no unseeded randomness (§6).
4. **Project views.** The world computes one view per user from `state[t]` using the rules in §2. Clients and functions never compute views.
5. **Publish.** Append the tick record to the replay log (§6). Send each user their view, the presentation channel its render-shaped copy (§5), and each user function its view together with the submission endpoint for tick `t+1`.
6. **Present.** Clients render from their view. Interpolation between ticks is local to presentation and never feeds back into the world.

The Sonic and Tails case falls out directly. Sonic's entity takes its effective intent from the human input channel; Tails's entity takes it from the function channel. Both resolve in step 2 of the same tick and step together in step 3.

## 2. View-projection rules (anti-cheat boundary)

- **A projection, not a reference.** The world serialises a fresh per-user document each tick. It holds only what that user may know: their own entity (position, facing, field-of-view radius), the entities visible to them (`id` and position, for entities passing the field-of-view, line-of-sight and team-knowledge checks), and world-provided extras such as scores and timers. The document holds no object references, entity handles or indices into world state.
- **Visibility is computed by the world.** Line-of-sight uses world geometry on the server. Occluders and hidden entities are never sent, so a client or function cannot read what its view does not contain. Field-of-view, line-of-sight and team knowledge are pure functions of `(state, entity_id)`, so a view can be re-derived for audit (§6).
- **Composition with the capability model.** The view is not itself a capability. It is data, delivered by a subscription capability, and the capability gates only actions. Two capability families extend `RocCap.roc`:
  - `world.view`, a subscription. Each tick the server pushes the caller's projected view as the subscription payload. The document carries a `view_version`, so an incompatible function fails loudly rather than silently.
  - `world.intend`, a request. The function submits its intent document for tick `t+1`. The server answers `ok: true` on acceptance, or `ok: false` with `error` set to `late`, `malformed` or `entity_not_owned`.
- The existing `RocCap` bookkeeping (`issue`, `attach`, `requests`, `done`, `from_pending`) applies unchanged. A function that crashes mid-tick reattaches its pending intent and resubmits.
- No `world.read` capability exists. A function cannot turn a view into a wider read of world state, because the only data it receives is already projected.

## 3. Intent schema, validity window and late-intent fallback

An intent is submitted against the view of tick `t-1` and is effective for tick `t`:

```json
{
  "intent_version": 1,
  "tick": 1247,
  "entity": "ent_tails_02",
  "action": { "move": { "dx": 1, "dy": 0 } },
  "expires_tick": 1247
}
```

- **One slot per `(entity, tick)`.** `expires_tick` equals `tick`. An intent is valid for the single tick it names and is never carried forward implicitly.
- **Late intents** are those that arrive after step 1 of tick `t` closes, or that name a past tick. The server rejects them with `error: "late"`, and the entity takes the fallback chain below in order:
  1. Repeat the entity's last effective intent (momentum default: a runner keeps running, a follower keeps following).
  2. If the entity type declares `on_missing: "hold"`, stand still instead (turrets, UI cursors).
  3. A human input for the same tick always fills the slot first (§4), before any fallback.
- **The fallback is logged.** The effective intent is written to the replay log with `source: "fallback:<rule>"`, so replay reproduces it without needing the original arrival timing.
- **Function timeout** is an ordinary late intent. A timed-out function has no submission at the deadline, and the fallback chain applies. The tick loop has no separate timeout path. The mechanism that produces the timeout belongs to #5966.

## 4. Blending: human input and function intent

- **Default: human input wins.** When a human input and a function intent name the same entity in the same tick, the human input is effective. The function intent is logged with `source: "superseded_by_human"`.
- **Delegation inverts the default, explicitly.** An entity may be marked `delegated: true` in world configuration; this is the Tails case. For a delegated entity the function intent wins, and any human input for it is logged with `source: "superseded_by_delegate"`. Delegation is set per entity by world configuration and is visible in the view. It never results from a silent per-tick race.
- **Reserved blending field.** Each entity carries `blending: "human_wins" | "delegate_wins"`, which encodes the two rules above. A future rule for two functions sharing one entity needs an explicit voting rule in world configuration. That case is out of scope for v1.
- **No averaging of continuous inputs.** One source wins whole. Averaging two sources yields a third, usually worse, input (a runner steering halfway between two good paths into a pit).

## 5. Presentation and function view: one projection, two fidelities

The world computes a single projection per user. It is not two projections owned by different layers.

- The function channel receives the projection whole, because a function needs exact positions to decide. It also receives the intent submission endpoint.
- The presentation channel receives the same projection with render hints added (sprite ids, interpolation anchors, effect triggers) and function-only fields removed (intent endpoint tokens, expiry bookkeeping).

Owner: the world (#5955 program). Layers choose how to shape the projection for their channel, but they do not recompute visibility. This keeps "what the player sees is what the function saw" literally true. It also lets spectator mode and post-match review use the same projection: a spectator is a user with no entity and a wider field of view.

## 6. Replay and determinism (#5779)

- **Replay log per tick:** `(tick_id, view_version, intent_version, effective_intents[t] with source, state_hash[t])`.
- **Views are not logged.** They are re-derived from `(state[t-1], effective_intents[t])` through the pure step and pure projection. Logging them would duplicate data and add a risk of divergence.
- **Determinism obligations on the world.** Step and projection are pure in `(state, intents, tick_id)`. They read no wall clock (the tick id is the clock), use only seeded randomness (seed in world configuration, counter in state), and do not depend on iteration order. These are instances of the nondeterminism audit in #5779; this design consumes that infrastructure and does not duplicate it.
- **Replay check.** Re-running the log through step and projection must reproduce every `state_hash`. A mismatch is a bug. Any view a function claims to have acted on can be compared with the re-derived view for that tick, which makes the anti-cheat boundary auditable after the fact.
- **Testing hooks.** The intent log is a ready-made corpus for seeded fuzzing under #5779. Mutated intents should give stable hashes for identical inputs and graceful fallback for malformed ones.

## 7. What this extends, and what it needs from siblings

- **Extends** `RocCap.roc` with `world.view` (subscription) and `world.intend` (request). The existing bookkeeping functions need no change.
- **Needs from #5967:** the binding and fuel-metered invocation model that produces the intent for a binding.
- **Needs from #5966:** the timeout and fuel cutoff mechanism that determines when a submission counts as late.
- **Needs from #5779:** the simulated-clock seam and the replay harness.
- **Needs from #5962:** the world per-tick budget and what it guarantees across ticks.
- **Needs from #5955:** the world configuration schema for `delegated`, `on_missing` and `blending`.

## 8. Open questions

- Whether `view_version` should be negotiated at subscription time or fixed per world. Recommendation: fixed per world, so every user in a match sees the same schema.
- The exact field-of-view and line-of-sight geometry is world-specific and is decided by each world, not by this record.

## References

- Issue [#5968](https://github.com/developing-today/code/issues/5968) and its comments, including the 2026-11-22 decision.
- Parent [#5955](https://github.com/developing-today/code/issues/5955); replay [#5779](https://github.com/developing-today/code/issues/5779).
- [Per-user function mapping](../2026-11-08T10-26-00Z_design_id_per_user_function_mapping/2026-11-08T10-26-00Z_design_id_per_user_function_mapping.md).
- `pkgs/id/examples/roc-world/RocCap.roc`.
