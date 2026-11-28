# World capability ceilings

See the [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md), the [durable resource profiles](../2026-10-16T10-30-00Z_feature_durable_world_resource_profiles/2026-10-16T10-30-00Z_feature_durable_world_resource_profiles.md) record, and issue #230.

## Intent

Let the server and each world narrow which named host capabilities a world program may use, without changing what a grant means. A grant says what an admin allowed. A ceiling says how far the server or the world is willing to let grants reach. A ceiling never adds authority, and the only way to raise a ceiling is to change it.

This slice authorizes named host providers only. It does not add Wasm imports, filesystem, network, or process access, and it does not touch the import-free Wasm validator, native enablement, or the seccomp/OS sandbox.

## Effective authority

For a capability `c`, a use is allowed only when all four hold:

```
c ∈ CATALOG
∧ c ∈ server_caps          (serve --world-allowed-caps; default: CATALOG)
∧ c ∈ world_ceiling        (durable per world; default: CATALOG)
∧ (c ∈ granted ∨ "*" ∈ granted)
```

`*` is shorthand for the grant set. It never bypasses either ceiling. The report's `effective` list is `granted ∩ server_caps ∩ world_ceiling ∩ CATALOG`, with `*` expanded to the allowed names.

## Decisions

- **Server ceiling.** `WorldLimits.server_caps` holds the set and defaults to every catalog name. The `serve` flag `--world-allowed-caps <names>` takes a comma-separated list, validates each name with `validate_ceiling_names`, and refuses `*`. It requires `--world`, like `--world-cap-policy`. A `--world-cap` name outside the server ceiling is logged at startup, since the program will be refused it. The server ceiling is runtime configuration and is not journaled. It applies to every world the process opens, and it takes effect on restart; lowering it needs no journal rewrite.
- **World ceiling.** A durable whole-value set in `WorldCore`, journaled as `JournalEntry::CapsCeiling { ceiling }`. Checkpoints carry it as `caps_ceiling: Option<Vec<String>>`, which is omitted when it is the full catalog. Journals and checkpoints without it restore the full catalog, which is the prior behaviour.
- **Ledger.** `CapLedger` holds `allowed`, which is the intersection of the server and world ceilings within the catalog. `WorldCore` recomputes it whenever either changes (`sync_ledger_ceiling`), including after restore. `decide_for` checks, in order: unknown name, then ceiling, then actor scope, then grant, then grant-on-use. A name outside the ceiling is refused with `CapError::OutsideCeiling` (code `outside_ceiling`) and counted as denied. Grant-on-use only runs after the ceiling check, so it never adds an excluded name.
- **Grants are kept.** A ceiling change never rewrites `granted`, so lifting the ceiling restores the earlier authority.
- **Compile gate.** `check_imports` is unchanged in logic. Callers pass the effective list, and `compile_and_install` now reads `CapsReport.effective` where it used to read the raw grants. A refused import now says the world "does not allow" it and names a ceiling as a possible cause.
- **Subscriptions.** `reconcile_subscriptions` runs after every ceiling change and after `update_caps`. An armed subscription that is no longer granted is dropped and its tick disarmed, so the next pass refuses it (`denied_subs`, with `outside_ceiling` or `cap_denied`). Every refused subscription is also dropped and decided again, so a refusal that still holds is announced again, and one that a change now grants is armed.
- **Admin operation.** `WorldCommand::SetCapsCeiling`, `ClientFrame::SetCapsCeiling { admin_token, ceiling }`, and `WorldService::set_caps_ceiling`. The service checks `authorize_admin`, exactly as `UpdateCaps` does. The frame is not added to `frame_admin_token`, which would let it create a world. The reply is `{"type":"caps_ceiling","report":…,"message":…}`, and the message says the ceiling applies immediately. `WorldClient::set_caps_ceiling` mirrors the profile client. No `id world` CLI command was added.
- **Report.** `CapsReport` gains `ceiling` and `effective`, both `#[serde(default)]`, so a report from an older server still deserializes. `policy` and `catalog` change from `&'static str` to owned `String`s so the type can derive `Deserialize`. The wire JSON is unchanged. `missing` is now `requested − effective`, so a requested capability blocked by a ceiling is reported as missing.

## Behaviour changes to existing paths

Both were found while reading the tick path and are fixed by the same reconcile step:

- A revoke through `update_caps` now stops an armed subscription that the revoke removed. Before, `fire_ticks` never checked the grant, so ticks kept firing after a revoke.
- A grant now unblocks a refused subscription on the next pass. Before, a refused subscription stayed in `active_subs` and was never decided again, so the `denied_subs.clear()` in `update_caps` had no effect.

## Validation

- `cargo fmt --check` is clean.
- The `world` filter (`sandbox,world`) passes 242 of 242. The full lib suite passes 1311, with 0 failed and 4 ignored.
- The sandbox regressions `sandbox::tests::rejects_modules_with_host_imports` and `world_native::tests::the_seccomp_allowlist_kills_escaping_workers` pass.
- The test build with `--features world` and without `sandbox` compiles. Its one warning (an unused `RngExt` import in `world_session.rs`) predates this change.
- Clippy reports no warnings on any line this change adds or edits.
- Tests cover: the ledger's ceiling rules and refusals; journal replay, checkpoint carry-over, and the old-journal default; the server ceiling bounding a restored grant; a ceiling change denying a prior grant; a lowered ceiling stopping and refusing an active tick subscription, and a raise resuming it; the compile gate and `compile_and_install` using the effective list; admin-only refusal over the session and iroh; the protocol reply; an old report still deserializing; and a store restart that installs `lounge.wasm` and reopens under a lower server ceiling and under the default.

## Implementation boundary

Implemented as specified: the effective-authority formula, the server flag, the journaled and checkpointed world ceiling, the ledger intersection, the compile gate, subscription refusal, the admin-only immediate operation, and the additive report fields. Out of scope: Wasm imports, native enablement, the seccomp/OS sandbox, and the CLI.

## Rollback caveat

Rollback is not safe without a backup. A binary built before #244 treats an unknown entry as a torn append when it is the last line of the journal, and truncates it. That silently drops the newest ceiling change. An unknown entry that is not last makes such a binary refuse the journal. Compaction writes `CapsCeiling` before the checkpoint, so a compacted journal refuses, but the first change after compaction can be last. Restore a pre-change journal backup before rolling back. Binaries built after #244 refuse any complete line they cannot read. This is the limitation tracked in issue #227.

## Follow-ups

- A client CLI command, such as `id world caps --ceiling`.
- A `*`-free default ceiling for new worlds, so a new world starts with an explicit list instead of the full catalog.
- Per-world ceiling management in the UI.
- The compile gate reads the effective list before the install takes its lock. A ceiling change between the two can therefore admit a module that the new ceiling excludes. Runtime enforcement still refuses the excluded use, so this is not an escalation.

## References

- [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md)
- [Durable per-world resource profiles](../2026-10-16T10-30-00Z_feature_durable_world_resource_profiles/2026-10-16T10-30-00Z_feature_durable_world_resource_profiles.md)
- `pkgs/id/src/world_caps.rs`
- `pkgs/id/src/world.rs`
- `pkgs/id/src/world_compile.rs`
- `pkgs/id/src/world_session.rs`
- `pkgs/id/src/world_net.rs`
- `pkgs/id/src/world_store.rs`
- `pkgs/id/src/commands/serve.rs`
