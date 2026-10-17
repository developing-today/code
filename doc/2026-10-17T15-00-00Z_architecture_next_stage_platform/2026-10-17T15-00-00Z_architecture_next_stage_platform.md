# Next stage of the platform: foundations, then flexibility

See the [Phase 11 plan](../../.opencode/plans/phase-11-foundations.md) and the [Phase 12 plan](../../.opencode/plans/phase-12-platform-flexibility.md).

## Intent

Phases 9 and 10 delivered portable screens, structured Roc libraries, per-world execution profiles, capability ceilings, roles, fixtures, and scheduled events. They were delivered faster than they were verified. This record sets the direction for the next stage: first make the platform trustworthy and measurable (Phase 11), then make it flexible enough that other people extend it without changing its core (Phase 12).

## Decisions

- **Foundations before features.** The Nix check has no recorded success, and the failures found so far are test setup, not product logic. Every later task needs a signal it can trust, so CI repair comes first and is a prerequisite, not a parallel track.
- **Measure before optimising.** Fuel is the guests' main ceiling, and the first optimisation target (territory at about 71 percent of the limit) was found by hand. Budgets as tests make regressions visible and make optimisation work checkable.
- **One place decides what a client may see.** Three open issues (#240 seed, chat visibility, #242 host events) are the same question asked in different places. A single client projection answers it once, with a test per row.
- **Make the rollback rule into tooling.** Older binaries truncate an unreadable final journal line (#227, #244). A journal format version, a check, and a backup command turn a documented hazard into something enforced, and must land before any new journal entry type in Phase 12.
- **Extensibility through data and registries.** The provider registry and the world manifest turn core-code changes into declarations. This is the main flexibility lever, and the plans put them first in Phase 12.
- **Every new reach is ceilinged, audited, and off by default.** Providers such as `http.fetch` and `notify` widen what a guest can cause. They arrive behind the ceiling machinery from P9.3e, with a refusal test and an audit entry, and none adds a Wasm import.

## What is not decided here

The relay kit location (#238), seed secrecy (#240), chat visibility, whether to take the Firefox E2E work on (#229), and the choice between the Wasm component model and the current ABI as the long-term interface (F12.8). The Phase 11 plan lists the first three as decisions needed before work starts.

## Progress

- F11.6 per-call fuel metrics and budgets: in progress on `platform-perf-budgets`.

## References

- [Phase 11 plan](../../.opencode/plans/phase-11-foundations.md)
- [Phase 12 plan](../../.opencode/plans/phase-12-platform-flexibility.md)
- [Phase 9 and 10 plan](../../.opencode/plans/phase-9-portable-platform.md)
