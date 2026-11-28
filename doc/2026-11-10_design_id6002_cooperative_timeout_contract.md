# Contract: cooperative timeout — invocation budget, checkpoint convention, hard-kill outcomes

- **Date:** 2026-11-10
- **Status:** contract specification — no implementation is included in this change.
- **Issue:** [#6002](https://github.com/developing-today/code/issues/6002), "id: design: cooperative timeout contract — deadline passing, checkpoint convention, hard-kill semantics". Queue task 13794.
- **Programme:** [#5955](https://github.com/developing-today/code/issues/5955) (worlds design program). Parent topic: [#5966](https://github.com/developing-today/code/issues/5966) (gas/timeout research).
- **Aligned with, not re-deciding:** the per-user function mapping design [#5967](https://github.com/developing-today/code/issues/5967) (landed, `doc/2026-11-08T10-26-00Z_design_id_per_user_function_mapping/`) owns the mapping and its failure table; this contract adopts that table's timeout and fuel-exhausted rows verbatim (§4) and specifies the contract surface both sides code against. The tick/intent data model is [#5968](https://github.com/developing-today/code/issues/5968) (intent shape; the no-op/default intent).
- **Seams owned elsewhere (consumed, never decided here):** the concrete enforcement mechanism and the fuel-cost table are [#5966](https://github.com/developing-today/code/issues/5966) (research landed: `doc/2026-11-09T00-10-25Z_research_id5966_gas_timeouts_multithreading/` — Wasmtime fuel per binding/store as the primary bound, epoch interruption as a future backstop); budget interfaces are [#6001](https://github.com/developing-today/code/issues/6001) and the per-tick world budget is [#5962](https://github.com/developing-today/code/issues/5962) / #5967 §6. Where this contract needs a number those seams have not fixed, it names the seam instead of inventing the number.
- **Evidence baseline:** `origin/main` at `2867299652`, 2026-11-10. Verified at that baseline: issue #6002 is OPEN with no contract spec on the issue or in `doc/`; the #5967 design is landed and issue #5967 is CLOSED; current host metering is per invocation of the world's guest (one `SandboxLimits.fuel`, default 10,000,000), not yet per user function (#5966 research §1–§2) — this contract is written for the per-binding shape #5966 recommends and #5967 assumes.

## Contract summary

| Question | Contract |
| --- | --- |
| What an invocation receives | A budget bundle: **fuel budget** (the bound), an **optional wall-clock deadline** (cooperative, single-host backstop only), the tick identity, and the binding's **resume checkpoint** if one survives from a killed invocation (C1) |
| How a function watches itself | Host-provided self-observation: `fuel_used()` / `fuel_remaining()` and, where a deadline exists, `time_remaining()` — the Screeps property the issue cites: usage is self-observable, so cooperation is possible (C1) |
| What a checkpoint is | A function-initiated, host-mediated save of **function-local progress** (opaque state blob + monotonic sequence), taken at the function's own yield points. A checkpoint is **not** a world-state commit and never extends the budget (C2) |
| What a kill observes | Exactly the #5967 failure-table row: **default intent, pre-tick state (no commit), strike +1**, plus an outcome record naming the kill kind (`fuel-exhausted` or `timeout`), fuel spent, and the checkpoint offered to the next invocation (C3) |
| Partial results | **Discarded.** No partial intent, no partial state commit, ever. The only survivor of a killed invocation is its last checkpoint, and it revives as *input* to a later invocation, not as effect (C3) |
| Repeat offenders | The #5967 strike discipline (consecutive failures, success resets, suspension at the world-configured limit, default 3), surfaced to the world author as queryable outcome records and a suspension event (C4) |

## C1 — Invocation: the budget bundle and self-observation

- Every invocation of a user function — `init` or `step` in the #5967 interface — is made **with** a budget bundle, fixed before the call and immutable during it:
  - `fuel_budget` — the per-invocation fuel the binding may spend (sized by the budget interfaces, #6001/#5962; spent against #5966's cost table; enforced by #5966's mechanism). This is the bound that appears in adjudication.
  - `deadline` — **optional**, and present only where the host runs the cooperative single-host profile (#5967 rule C4): a wall-clock instant after which the backstop may fire. Cross-host invocations never carry one, and nothing in the outcome record of a cross-host invocation may cite wall-clock.
  - `tick` — the tick identity this invocation serves, so a function can tell a fresh tick from a resumed one.
  - `resume` — the binding's surviving checkpoint (C2), or none.
- **Self-observation is part of the contract, not a courtesy.** The host provides the function two reads, chargeable at zero or seam-fixed fuel: `fuel_used()` (equivalently `fuel_remaining()`) for the current invocation, and `time_remaining()` where a deadline was passed. A function that cannot observe its own consumption cannot cooperate, and a contract that asks for cooperation without observation is unenforceable in practice — this is the Screeps `Game.cpu.getUsed()` property the issue takes as its model.
- The bundle is **per invocation, not per binding-session**: budgets do not accumulate across ticks, unspent fuel does not roll over, and a function receives no promise about future bundles beyond the world's published budget policy (#5962 seam).

## C2 — Checkpoint convention: progress, never commitment

- A checkpoint is a **function-initiated** save, made by calling the host's checkpoint operation at a point of the function's own choosing, carrying:
  - `state` — an opaque blob of function-local progress, bounded by a size cap (the cap is a budget-interface number, #6001 seam; the existing message-size cap family is the model, not the decision);
  - `sequence` — a host-assigned, strictly increasing per-binding sequence number, so a stale checkpoint can never be mistaken for a newer one;
  - host-stamped `fuel_spent` at the moment of the save (stamped by the host, never asserted by the function).
- **Semantics — the three rules both sides code against:**
  1. **A checkpoint is not a commit.** World-visible state changes only through the intent a `step` returns and the mapping accepts (#5967/#5968). Checkpoint state is invisible to the world, to other bindings, and to adjudication.
  2. **A checkpoint does not buy time.** Taking one costs whatever the seam charges and never extends `fuel_budget` or `deadline`. Checkpoint frequency is the function's own trade-off: more checkpoints, less lost progress per kill, more fuel spent saving.
  3. **A checkpoint survives exactly one kill.** On a hard kill (C3), the last checkpoint taken during the killed invocation replaces any older one and is offered as `resume` to the binding's next invocation. A successful invocation **consumes** it (the offer is not repeated), and a checkpoint taken during a successful invocation is discarded at tick end unless the function explicitly carries it forward as its closing action — the default is that checkpoints are kill insurance, not a persistence channel (persistence is binding-owned state under #5967, a different mechanism).
- A function is **never required** to checkpoint. The convention exists so a long-running function *can* degrade to a defined intent with bounded rework, per the issue's framing; a function that ignores checkpoints simply restarts from its binding state after a kill.

## C3 — Hard-kill semantics: what the world observes

- A **hard kill** is the backstop firing: fuel exhaustion (the portable bound) or, in the cooperative profile only, the wall-clock deadline (the #5967 timeout row). The kill is immediate and not appealable from inside the invocation; there is no kill warning callback, because a callback at the boundary would itself need a budget.
- The observable outcome is **exactly** the #5967 failure-table row for the kill kind, restated here so function authors see it in the contract they code against:
  - **Intent:** the world's **default intent** for the entity (world-declared, else the #5968 no-op intent).
  - **State:** **pre-tick state — no commit.** Nothing the invocation did to world-visible state exists, including work done before its last checkpoint (checkpoints are function-local, C2 rule 1).
  - **Discarded, explicitly:** any partially constructed intent; any host work product of the invocation beyond the charges already made (charges for host work actually performed stand — they price work done, per the #5966 host-side accounting seam — but buy no state).
  - **Strikes:** strike +1 (§C4).
  - **Outcome record:** the host records `{ binding, tick, kind: fuel-exhausted | timeout, fuel_spent, checkpoint_offered: sequence | none }`. This record is the *only* narrative of the kill; in particular a cross-host record never carries a wall-clock kind (C1).
- A kill is **not a crash**: it carries no trap class, is not flagged as a contract violation, and does not by itself say anything about the function's correctness — only about its cost against its budget.

## C4 — Repeat offenders: strikes and surfacing

- Strikes follow #5967 verbatim: a per-binding counter of **consecutive** failure rows in the fuel-exhausted / timeout / crash family; a success resets it to zero; at the world-configured limit (default **3**) the binding is **suspended** — it stops being invoked and takes the suspended row until the user switches or repairs the function. Suspension is protective, not punitive (#5967), and this contract adds no second penalty layer.
- **Surfacing (the contract's addition, within its own scope):**
  - Outcome records (C3) are **queryable by the world author per binding**: kill kind, fuel spent, strike count, and suspension state are author-visible facts, not host logs.
  - Suspension emits an **event** naming the binding and its last failure row (the #5967 rule), delivered through the world's existing author-facing surfaces; the user whose function was suspended sees the same record — a silent suspension is a contract breach by the host.
  - A function author can always answer "why did my function stop running" from the outcome records alone: last kill kind, fuel spent against which budget, and the strike history. If a record cannot answer that, the record is incomplete, not the question.

## 5. Invariants both sides must hold

1. **Fuel is the only adjudicable bound.** Wall-clock appears only in the cooperative profile, only as a backstop, and never in adjudication or cross-host outcome records (#5967 C4).
2. **No commit except through a returned intent.** Checkpoints, host work products, and partial execution never mutate world-visible state (C2/C3).
3. **A checkpoint never extends a budget** and is consumed by the next invocation, kill or success (C2).
4. **Every kill produces exactly one outcome record** in the C3 shape; there is no kill without a record and no record without a kill.
5. **Self-observation is always available** during an invocation (C1); a host that cannot report `fuel_used()` is not running this contract.
6. **Strikes count consecutive failures only**, reset on success, and suspension is always visible to both the world author and the function's user (C4).

## 6. Alternatives rejected (summary)

- **Checkpoint-as-commit** (a checkpoint persists world state mid-tick) — breaks the #5967 pre-tick-state rule and makes every kill a partial-effects problem; rejected by C2 rule 1.
- **Budget extension on checkpoint** ("checkpoint to buy another slice") — turns the bound into a negotiation and makes fuel spent unaccountable per tick; a function that needs more budget needs a bigger bundle next tick, under the world's budget policy.
- **Partial-intent salvage** (apply the intents computed before the kill) — the intent is only meaningful as the function's complete answer for the tick; a prefix of it is a different, unreviewed decision. Discarded (C3).
- **Wall-clock as the primary bound** — non-deterministic across hosts; rejected by #5967 C4 before this contract was written.
- **Kill-warning / graceful-shutdown callback** — unbounded work at the exact moment the bound fired; the checkpoint convention is the graceful path, taken *before* the boundary.
- **Silent suspension after repeated kills** — an author and a user who cannot see why a function stopped will route around the platform; surfacing is contractual (C4).

## What this contract does not do

No code, no ABI encoding for the budget bundle or the checkpoint blob beyond the field vocabulary above, no fuel numbers, no cost table, and no enforcement mechanism — those are the #5966, #6001, and #5962 seams named in the header. Implementation (per-binding stores, the checkpoint operation, outcome-record storage and surfacing) is follow-on work to be filed from this contract; issue #6002 stays open until this contract is merged, per its acceptance.
