# World isolation modes

## Plan

Worlds on one `id serve` host are fully isolated by default: no world can read
or write another. An admin or moderator may mark a world **unisolated** so that
its directory can be read by members of other unisolated worlds. Writes never
cross worlds.

### Semantics

- **`Isolation::Isolated`** (default; absence on disk means this). No cross-world
  reads or writes in either direction. Unknown and foreign names stay
  indistinguishable to outsiders.
- **`Isolation::Unisolated`**. The world's directory may be read by a session in
  another world, but only when *both* worlds are unisolated. An isolated world is
  never readable from outside. Writes never cross worlds.
- Same-world access is unaffected by isolation.

### Authority

- New `Level::Moderator`, between `Manage` and `Admin`. Existing levels keep their
  names, order and serialized forms.
- Setting isolation requires the process admin token, or a signed request whose
  key belongs to an account holding `moderator` or `admin` in at least one group of
  the world's directory. Anything else is refused as forbidden.
- `Moderator` does not satisfy any `Admin` check (group structure, visibility,
  deletion). Existing `Manage` checks now also admit `Moderator` by ordering, but
  `set_member` treats a non-admin actor under the manager rules, so a moderator
  gains no grant or membership power beyond a manager's.

### Storage and replay

- A new journal entry, `Isolation { isolation }`, written whole on each change, the
  same pattern as `CapsGranted`. Replay restores it; compaction carries it into the
  checkpoint header, next to `CapsGranted`, so trimming the journal keeps it.
- Only `Unisolated` is written by compaction, so worlds that never change the
  setting have the same journal as before.

### Enforcement point

No cross-world read path exists yet. The single decision point is
`WorldHub::authorize_read(reader, owner)` backed by the pure rule
`cross_world_read_allowed`. It is tested for the default-deny cases now, and
becomes the gate for any future cross-world directory read.

### Interface

- Frame `set_isolation` (first frame, like `create_world`): carries
  `isolation`, optional `admin_token`, or optional `signed` (a
  `directory_auth::Signed` over method `PUT`, path `/world/<name>/isolation`, body
  = the isolation word).
- CLI: `id world isolation <NODE> <isolated|unisolated> --admin-token T [--world NAME]`.
  Matches the existing `create` / `caps` style (`--world`, not a positional name).

### Non-goals

- A CLI for signed moderator requests (protocol supports it; a client is deferred).
- Listing isolation in `id world list`.
- Any cross-world read transport.

---

## Results

Results are appended below once the work is complete.

---

## 2026-10-10T04-13-23Z Update: Results

Built as planned. Deviations are listed below.

- **Level**: `Moderator` sits between `Manage` and `Admin`. `levels_keep_their_order_and_wire_names` pins order and wire names.
- **Storage**: `JournalEntry::Isolation` is written on each change, restored on replay, and carried into checkpoints. Absence means `Isolated`.
- **Set**: `WorldCommand::SetIsolation` and frame `set_isolation`. Accepts the admin token or a signed request; anything else is `Forbidden`. Journaled before the in-memory change.
- **Read gate**: `WorldLease::authorize_read` over `cross_world_read_allowed`. Default deny. No caller yet.
- **CLI**: `id world isolation <NODE> <isolated|unisolated>` with `--world`, `--admin-token`, `--addr`, `--no-relay`.

Tests (`cargo test --lib --features world`): 651 passed, 3 failed. The 3 failures are `world_compile::tests::*`, all `examples/roc-world is not a world platform directory` (`host.wasm` not built). No serve test failed.

New tests: `world::isolation_tests` (default, moderator and admin set, others refused, checkpoint keeps the setting), `world_hub::tests` (truth table, `authorize_read` transitions), `world_store::tests::isolation_survives_a_restart`, `world_session::tests::a_signed_moderator_sets_isolation_and_a_forged_request_does_not`.

Checks: `cargo fmt --check` clean. Clippy (`--all-targets --all-features`, rustc 1.97.0) exits 0. New warnings are the `map_err(|_| …)` pattern already used across the handles.

Deviations:
- **Moderator scope is world-wide.** Any account holding Moderator or Admin in any group of the world's directory may set isolation. This is the reading the plan settled on; it means any group admin can change isolation. Needs review.
- **CLI uses `--world`**, not a positional name, to match `create` and `caps`.

Deferred: a signed-request CLI for moderators, isolation shown in `id world list`, and any cross-world read transport.
