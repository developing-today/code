# Phase 12: platform flexibility

See the [architecture record](../../doc/2026-10-17T15-00-00Z_architecture_next_stage_platform/2026-10-17T15-00-00Z_architecture_next_stage_platform.md). This phase builds on [Phase 11](phase-11-foundations.md); the dependencies are named per task.

## Goal

Make the platform something other people can build on without changing its core: new capabilities added as components, worlds described and reproduced as data, programs written in several languages and upgraded without losing state, and many worlds run on one node and across nodes, all with the same sandbox and authority rules.

## What "best in class" means here

These are the exit tests for the whole phase. Each is measurable.

| Criterion | Target |
| --- | --- |
| Time from empty directory to a running world a friend can join | Under 5 minutes, one command per step, documented |
| Adding a host capability | A new provider and its manifest, no edits to `world.rs` |
| Guest languages with a passing conformance suite | At least 3 (Roc plus two of Rust, TypeScript or AssemblyScript, Zig) |
| Upgrading a running program | State preserved across versions, failed upgrade rolls back, proven by a test |
| Idle worlds on one node | 1,000, with bounded memory per world |
| Active worlds on one node | 100 worlds at 20 events per second each, with p99 command latency under 50 ms |
| Reproducing a world | `world.toml` plus a module hash recreates grants, ceilings, profile, schedules, and invites |
| Bandwidth for a typical redraw | Under 10 percent of the full-frame size (baseline from F11.9) |
| Authority and sandbox | No new path to files, network, or processes except through a named, ceilinged provider |

## Workstreams

Tasks are numbered F12.n.

### A. Worlds as data

- **F12.1 Declarative world manifest.** A `world.toml` that names the module (hash or source), seed, grants, ceilings, resource profile, roles and invites, and schedules. `id world apply` reconciles a live world to the file, `--dry-run` prints a plan, and `id world diff` shows drift. Applying is idempotent and journaled, and every step uses the existing admin operations. Depends on F11.11 and F11.12. Exit: apply twice and the second run changes nothing; drift is detected after a manual `caps` change.
- **F12.2 Forking and templates.** `id world fork` creates a world from another's module and snapshot with provenance recorded, and templates are forks marked reusable. The install seed policy from F11.11 decides whether a fork may inherit the seed. Exit: a fork replays to the parent's records at the fork point.
- **F12.3 Roles v2.** Journaled grants for builder (install modules) and moderator (revoke participants) as scope bits, with an audit record for each use. Builder never receives network, file, or process authority. Depends on F12.1. Exit: a builder installs a module, a moderator revokes a participant, and a guest cannot do either.

### B. A real extension model

- **F12.4 Provider registry.** Capability providers become components with a manifest: name, version, argument schema, bounds, required scope, and default ceiling. The ledger, the compile gate, and the CLI read the registry, so the catalog stops being a hard-coded list. Depends on F11.10 and F11.13. Exit: a test provider is added without editing `world.rs`, and it appears in `caps`, the ceiling, and the compile gate.
- **F12.5 First new providers.** `kv` (a bounded, durable, per-world key-value store), `blob` (content-addressed, over iroh-blobs), `http.fetch` (admin allowlist, size and rate bounds, no redirects off the list, off by default), and `notify` (user-visible, rate limited). Each ships behind a ceiling and with an audit entry per use. Exit: each has bound, refusal, and replay tests; `http.fetch` has a test that proves an unlisted host is refused.
- **F12.6 Observability and audit.** A structured audit log of administrative actions and provider use, with OpenTelemetry export, building on F11.7. Exit: every admin frame produces an audit line, and a test replays the log to the final grants.

### C. More languages, stable contract

- **F12.7 ABI specification and conformance suite.** Write down the `plaza_*` exports and the capability protocol as a versioned specification, add a `plaza_abi_version` export, and publish a conformance suite built on fixtures from F11.16 and the recording work (#239). Exit: the Roc examples pass the suite unchanged.
- **F12.8 Second and third guest SDKs.** A Rust SDK for `wasm32-unknown-unknown` and one of TypeScript (AssemblyScript) or Zig, each with `screen`, `json`, `grid`, and `cap` helpers equal to the Roc libraries, and an example world. Evaluate the Wasm component model and WIT as the long-term interface, and record the decision. Depends on F12.7. Exit: each SDK passes the conformance suite and ships a playable example.
- **F12.9 Program upgrade with state migration.** Versioned snapshots and an optional `migrate` export. An upgrade runs on a probe instance first (the way checkpoints already do), commits only if the migrated state round-trips, and otherwise rolls back. Exit: a test upgrades a counter across a snapshot format change and a deliberately failing upgrade leaves the old program running.

### D. Scale

- **F12.10 Fair scheduling and quotas.** A hub scheduler that shares fuel and CPU fairly across worlds, with per-account quotas for worlds, storage, and fuel per second, and back-pressure for slow clients. Depends on F11.7 and F11.8. Exit: a world that spins on fuel cannot raise another world's p99 latency by more than 10 percent, shown in a benchmark.
- **F12.11 Delta and binary frames.** Screen patches as cell runs, an optional binary encoding, optional compression, and resume tokens so a reconnecting client receives a patch, not a snapshot. Baseline from F11.9. Exit: the bandwidth target in the table above, with a parity test that a patched client equals a full-frame client.
- **F12.12 Many worlds per node.** Lazy world open, idle eviction with checkpoint, and a bounded memory budget per world. Exit: the 1,000 idle and 100 active targets, measured by the F11.8 harness.

### E. Across nodes

- **F12.13 Journal replication.** Read replicas that follow a world's journal over a signed stream. A world keeps a single writer, so there are no write conflicts. Replicas serve spectators and survive the writer being offline. The rollback and format rules from F11.12 apply to the stream. Exit: a replica reproduces the writer's records, and a tampered stream is refused.
- **F12.14 Portals.** Cross-world messaging through a capability that both worlds' admins have granted, with bounded rate and size and an audit entry on each side. Depends on F12.4 and F12.6. Exit: two worlds exchange a message only when both sides have granted the portal.

### F. Authoring

- **F12.15 `id world dev`.** Watch the source, rebuild, and hot-swap into a scratch world using the F12.9 migration, with fixture replay on every change and a deterministic fuzz harness that drives a guest with generated inputs against declared invariants. Exit: an edit-to-running loop under 3 seconds for a small Roc app, and the fuzz harness finds a seeded bug.
- **F12.16 `id world doctor`.** Checks the toolchain, platform build, ceilings, profile, journal format, and clock, and prints the fix for each problem. Exit: each check has a failing-case test.

### G. Security hardening

- **F12.17 Trust and provenance.** Verify module signatures against an admin trust list, extend the observed provenance (#225) with a compiler version, and record the trust decision in the journal. This is trust, not only observation. Exit: an unsigned module is refused when the list is non-empty.
- **F12.18 Denial-of-service tests.** A suite that tries to exhaust fuel, memory, journal size, frame size, session count, schedule count, and provider bounds, and asserts each is refused with the documented code and that the node stays responsive. Exit: the suite passes in CI.

## Sequencing

```text
Phase 11 done: F11.1, F11.4, F11.10, F11.11
Still planned from Phase 11: F11.12 (journal format management)
F12.4 -> F12.5 -> F12.6 -> F12.14
F12.1 -> F12.2, F12.3
F12.7 -> F12.8 -> F12.9 -> F12.15
F11.7/8/9 -> F12.10, F12.11, F12.12
F12.13 after F11.12 and F12.6
F12.16, F12.17, F12.18 anytime after F11.10
```

F11.12 stays **Planned**, as the Phase 11 status tracker (`.opencode/plans/2026-10-20_phase-11-status.md`) records: the fail-loud rollback check landed (#244), but `id world journal check` and `id world journal backup` do not exist yet. The sequencing block above previously listed F11.12 as done; that was the inconsistency #841 reconciles — the F11.12 dependencies below (F12.1, F12.13, and the compatibility risk) are unchanged and still gate on it landing.

The provider registry (F12.4) and the manifest (F12.1) have the highest return, because each turns a core-code change into data.

## Risks

- **Scope.** Eighteen tasks is too many to run at once. Treat F12.4, F12.1, and F12.7 as the first slice, and decide on the rest after the exit tests for those three.
- **Authority creep.** `http.fetch`, `notify`, `blob`, and portals each widen what a guest can cause. Each must ship off by default, behind a ceiling, with an audit entry, and with a refusal test. None may add a Wasm import.
- **Compatibility.** Every new journal entry is forward-only for older binaries (#227). F11.12 must land before the first Phase 12 entry type.
- **Hidden state.** Hot upgrade and forking both copy or migrate program state. The seed and visibility decisions from F11.11 apply to both.

## References

- [Phase 11 plan](phase-11-foundations.md)
- [Architecture record](../../doc/2026-10-17T15-00-00Z_architecture_next_stage_platform/2026-10-17T15-00-00Z_architecture_next_stage_platform.md)
- [Phase 9 and 10 plan](phase-9-portable-platform.md)
