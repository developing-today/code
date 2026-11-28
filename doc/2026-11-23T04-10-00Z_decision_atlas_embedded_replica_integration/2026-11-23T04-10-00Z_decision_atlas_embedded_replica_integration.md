# Decision: embedded-replica integration shape for the atlas core shards

- **Date:** 2026-11-23
- **Status:** Decision record; no implementation is included here.
- **Issue:** [#11213](https://github.com/developing-today/code/issues/11213)
  (slice 3 of [#6666](https://github.com/developing-today/code/issues/6666),
  follow-up of [#5755](https://github.com/developing-today/code/issues/5755)),
  scope item 1 — decide and record the integration shape.
- **Parent design:** `doc/2026-11-04T21-27-34Z_design_atlas_scaler_revision`,
  §5.5 (embedded replicas and the sync budget) and §6 (engine verification).

## Decision

1. **One sync owner per replica home.** A dedicated replica-sync component
   owns each replica: it is the only process that opens replica files for
   sync. Whether it ships as a mode of the serve process or a sibling
   daemon is an implementation choice; the single-owner invariant is not.
2. **The crate is chosen by engine match, gated by a spike.** The core
   shards are TursoDB-type databases (§6, verified 2026-11-04; restore
   record #5669). Turso's Rust reference (read 2026-11-22) offers three
   crates and instructs matching the crate to the database engine: `turso`
   for Turso-engine databases (with a `sync` feature: push/pull, local
   database bootstrapped from the remote on first run), `libsql` for
   libSQL-engine databases (embedded replicas). The default candidate is
   therefore the **`turso` crate, pull-only**, not the `libsql` crate.
   The `libsql` crate is adopted only if the stage-1 spike proves its
   replication works against a TursoDB-type primary; that must be tested,
   not assumed from the shared `libsql://` URL form (§6 makes the same
   point about engine evidence).
3. **Reads never open a syncing replica file from a second process.**
   Turso's documentation warns that opening an embedded-replica file
   outside the syncing process can corrupt it. The replica transport is
   therefore a new `PipelineTransport` implementation living in the
   process that owns sync. `LocalShardTransport` (rusqlite) remains for
   static, non-syncing local files only. This supersedes the earlier
   assumption that a synced replica home could simply be served through
   the `dir:` spec as plain files; a replica home is registered as
   replica endpoints instead. (Fallback if the spike proves quiesced
   file reads safe: `dir:` consumption with sync quiesced during reads —
   recorded here as a fallback, not the default.)
4. **Tier rule is fail-closed and pre-network.** Replica creation takes a
   `Tier` and refuses `Tier::Secret` with a named error before any
   credential or network use, mirroring `TierCeiling`'s posture (§5.5:
   secret stores are never replicated off their home node; staging-tier
   replicas: none).
5. **The sync crate is pinned and isolated.** Whichever crate wins sits
   behind the `PipelineTransport` seam at an exact pinned version. The
   `turso` crate is pre-1.0 (v0.8.3 stable, October 2026; v0.9
   pre-releases), so the seam must make re-pinning or swapping cheap.
   Turso's announced acquisition by Supabase (October 2026) is a watch
   item for the program, not an input to this decision.

## Rationale and rejected alternatives

- **`libsql` crate for sync + rusqlite reads of the same file** (the
  shape #11213's scope item 1 floated): rejected as the default on two
  independent grounds — engine mismatch per the vendor's own reference,
  and the second-process-open corruption warning in decision item 3.
- **`libsql` crate for sync and reads, bridged into the pipeline:**
  rejected. atlas-core's `PipelineTransport` is synchronous
  (`fn pipeline(&self, &[Stmt]) -> Result<Vec<Vec<Row>>>`,
  `crates/atlas-core/src/turso.rs`); the libsql crate is async. A
  block-on bridge would put a tokio runtime inside every consumer,
  including the standalone CLI's hot path, to use a libSQL-engine
  feature against TursoDB primaries.
- **Shelling out to the Turso CLI for sync:** rejected. No library
  contract, no in-process freshness or error surface, and the serve
  path would depend on CLI sync semantics meant for interactive use.
- **Hand-rolled Hrana/WAL delta sync:** rejected. It re-implements the
  vendor sync protocol against a moving pre-1.0 target.

One property of the current code makes the chosen shape cheap: local
scans already compute cosine in Rust (`cosine_distance`,
`crates/atlas-core/src/shards/local.rs`) instead of relying on server
vector functions, so replica reads through the seam need no engine SQL
extensions — the read contract is rows in, distances computed locally.

## Evidence

- Seam on main (`ced8e85f3`, read 2026-11-22): `PipelineTransport`
  synchronous trait (`crates/atlas-core/src/turso.rs:96`);
  `ShardTransport: PipelineTransport` (`shards/mod.rs:296`);
  `LocalShardTransport` over rusqlite (`shards/local.rs:63`);
  `StdConnector` maps `ShardLocation::File` to `LocalShardTransport`
  (`shards/mod.rs:386`); `Tier` / `TierCeiling::admits`
  (`shards/mod.rs:61`, `:153`). The workspace has no `libsql` dependency
  anywhere (grep over `Cargo.toml` files, 2026-11-22).
- Design §5.5 budget: laptop replica working set ≈ 4–6 GB; initial sync
  ≈ 5 GB ≈ 21% of one month's 24 GB sync allowance; steady-state delta
  sync ≈ write volume (≤ a few hundred MB/month); page/WAL-based deltas.
- Turso Rust reference, https://docs.turso.tech/sdk/rust/reference
  (page updated 2026-11-18, read 2026-11-22): three-crate table
  (`turso` / `turso_serverless` / `libsql`), the engine-match
  instruction for remote access, `turso` sync push/pull with first-run
  bootstrap, and the `libsql` feature table (`replication` = `core` +
  embedded-replica support; C compiler required for `core`/`replication`).
- Second-process corruption warning: Turso documentation as quoted in
  secondary coverage read 2026-11-22
  (https://codexskills.dev/skills/databases/libsql-turso/ and
  https://aidtoolstack.com/blog/edge-databases-turso-and-libsql —
  "Do not open the local database while the embedded replica is
  syncing. This can lead to data corruption."). The implementation
  spike must re-confirm this against the primary docs for the chosen
  crate before the read-path fallback in item 3 is ever used.

## Consequences and staged plan

#11213 stays open; its remaining scope is staged so each stage is a
lane-sized packet (three earlier lane parks on #11213 recorded the
45-minute small-fix packet as a defect against this effort-L scope —
staging is the fix):

- **Stage 1 — spike (gates the crate):** `turso`-crate pull from one
  real internal shard on the laptop; record initial sync size/time
  against the §5.5 budget; confirm a promoted row appears after a pull
  without restart. Optionally attempt `libsql` replication against the
  same primary and record the outcome either way. Re-confirm the
  second-process warning in the chosen crate's primary docs.
- **Stage 2 — transport:** replica `PipelineTransport` implementation
  in the sync-owning process, secret-tier refusal with a named error,
  fixture tests (no live secrets), standalone/hosted regression tests
  unchanged.
- **Stage 3 — discovery wiring:** replica endpoints in the serve config
  (slice-1 re-discovery consumes them); snapshot-id invalidation on
  replica rotation already lands via #6666 slice 1/2.
- **Stage 4 — live verification:** #11213's acceptance record — initial
  sync size/time, delta sync after a promotion, sync-budget counters
  re-checked against Turso's.

#6666's third acceptance criterion (persistent embedded replicas
covered) remains open until stage 4 lands; #5755's consumer-wiring
scope is otherwise complete (slices via #6661, #6662, #6664, #6665,
#5668, and #6666 slices 1–2).
