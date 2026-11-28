# Session recording to replayable fixtures

See the [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md) (Phase 10 sketch: "Session recording", and P10.2 under Phase 10 progress). Chat visibility is the open question in the [roles and directory record](../2026-10-17T08-00-00Z_feature_world_roles_directory/2026-10-17T08-00-00Z_feature_world_roles_directory.md#known-gap-join-only-participants-can-read-chat), and this slice does not change it.

## Intent

App authors can only test against a live world today, and a bug seen in a session cannot be reproduced exactly. This slice exports a world's retained history and its records to a file, and replays that file offline through the same module and the same seed, comparing the replayed records with the recorded ones.

Concretely:

- `id world record NODE --capability CAP [--out FILE]` joins a world with a capability and writes its fixture.
- `id world replay --module FILE.wasm --fixture FILE --seed N [--allow-partial]` runs the fixture in the sandbox, prints a one-line verdict, and exits nonzero when the records differ.
- `id world verify --module FILE.wasm --fixture FILE --seed N [--allow-partial]` (F11.16, #544) replays the fixture twice and compares the records after every event the program sees, prints a one-line verdict first, reports the first diverging record (sequence, key, and both values), and exits nonzero at the first divergence. Where `replay` checks one run against the header's records, `verify` checks two fresh runs against each other, so it does not trust the header and also catches a divergence that appears mid-stream and later reconverges.

## Format

JSON Lines. Line 1 is the header; each following line is one retained, sequenced event, in order.

```json
{"fixture":1,"world":"lobby","module_hash":"<hex>","current_sequence":7,"complete":true,"records":{"board":"XXXOO----","plays":5,"winner":"X"}}
{"sequence":1,"kind":"other","name":"program_installed"}
{"sequence":2,"participant_id":1,"kind":"chat","text":"gl hf"}
{"sequence":3,"participant_id":1,"kind":"input","hex":"30"}
```

- The header holds the world id, the active module's hash (never its bytes), the head sequence, `complete`, and the records read at the head. It never holds the seed.
- `complete` is true only when the history starts at sequence 1 and the host did not need to answer with a snapshot.
- A `system` line (`{"sequence":0,"participant_id":0,"kind":"system","event":"..."}`) is part of the format, but a live recording never produces one; see the limits below.
- Any other kind is written as `other` with its name. Only `program_installed` is replayable, and only as a boundary.
- Decoding is strict: an unknown `fixture` version, an unknown kind, an unexpected or missing field, a blank line, a malformed hex or JSON line, a sequence gap, or a window that does not end at the header's sequence is an error.

## Decisions

1. **Reading goes through a join.** A join is the only way to read sequenced events, so recording needs nothing a join capability does not already have. Spectators can record, and they can already read the same events.
2. **One consistent head.** Recording reads module info and then the records; both must report the same sequence, or it re-reads (three attempts). Records are paged with `records_sequenced`, which refuses pages from different sequences. The existing `records` keeps its behaviour.
3. **History from sequence zero.** The join uses `after: 0` and reads events until the event at the head, then stops. A snapshot reply means retention no longer reaches back to one; the fixture then holds the newest events that fit, and it is incomplete.
4. **Install boundary, refined.** The brief said to refuse any install boundary. Every installed world's first sequenced event is its own `program_installed`, so the literal rule would refuse every fresh recording. The rule implemented: a replay begins at the last install, which stands in for the fresh instance `seed` creates. Anything before that install must be chat or an earlier install, because a program never saw it. A reinstalled world's window therefore is refused, with the error asking for a window that starts after the last install.
5. **Module check.** The header's `module_hash` must equal the hash of the module passed to the CLI (`check_module`). A fixture that records no module cannot be replayed.
6. **Replay uses the live path.** Input and system events go through `WorldProgram::update`, the same call the actor makes, so Input keeps its UTF-8 check and a host event is delivered as participant 0. Chat is not passed to the program.
7. **Verdicts.** `Match` and `Mismatch` compare the header's records with the replayed ones as parsed maps, so key order and whitespace do not matter. A partial window, or a fixture with no records, is `NotCompared` with a reason. Only `Mismatch` exits nonzero.
8. **Partial replay needs an explicit flag.** An incomplete fixture is refused unless `--allow-partial` is set, and its result is never compared.
9. **Verify compares two replays, never the live world.** `verify` reuses the replay path twice under the same seed and compares step by step (`world_fixture::verify` / `first_divergence`). Reading `records` per step is safe: it is a pure function of state, and the sandbox resets fuel per guest call. A partial window verified with `--allow-partial` is still compared run-to-run; only the header comparison is skipped.

## Limits

- **Host events are not in the stream.** Presence changes, capability results, and ticks reach the program as host events, but the actor never retains them in the event stream, so a recording cannot include them. A program that consumes them (`territory` consumes presence) replays to different records, and the header's records check reports `Mismatch`. A fixture is a faithful replay only for programs that do not depend on host events. The check catches this, but the fixture cannot repair it. `verify` inherits this limit (#242): both of its replays omit host events equally, so verify can pass for a guest whose live behaviour depended on them — it proves the recorded fixture replays deterministically, nothing more.
- **Recording joins the world.** The recorder's own presence is delivered to presence-aware programs, after the recorded head. The recording is unaffected, and the live world sees a join, as `logs` already does.
- **Retention bounds complete recordings.** `WorldLimits::default()` retains 1024 events. A longer history yields a partial window; it replays only with `--allow-partial`, and its result is not compared.
- **A partial window replays from a fresh instance.** It usually does not match the live state. Including the program's snapshot at the window's start would make partial windows exact (follow-up).
- **No cursor on `record`.** The recorder always starts at zero, so it cannot produce a window that starts at a later install (follow-up, with `complete` reconsidered for install-start windows).
- **Only the sandbox replays.** Native-compiled modules and custom resource profiles are not replayed. A world with a custom profile may behave differently under default limits.

## Security notes

- **No authority added.** A fixture holds only what the recording capability can already read: sequenced events and records, through a join. It holds no seed, no capability, and no module bytes, only the module's hash.
- **The seed is not secret from joiners today.** The install event carries `seed` (`src/world.rs`, `ProgramInstalled`, with no `skip_serializing`), and the join path sends retained events whole, so a join capability can read the seed from its event burst. The fixture leaves it out and does not widen what joiners can read. Any future claim that the seed is secret needs the event serialization changed first (see follow-ups).
- **Chat is included.** Chat lines are in a fixture. Any join capability already reads them through `events_after`, so recording does not widen chat visibility.
- **Fixtures are not signed.** The header's records are a consistency check against the file, not an attestation. Someone who edits a fixture can make its records match.
- **Replay runs the module you name.** `--module` is the code that runs. Replay compiles it and runs it in the sandbox a live world uses: no imports, the same fuel and memory limits. The fixture supplies inputs and expected records only, and each input is bounded as a live input is. Replaying a module from an untrusted build runs untrusted guest code under that same sandbox. The brief framed this as untrusted fixtures running untrusted code; the accurate statement is that the module is the untrusted part.
- **Replay is offline.** It opens no network connection and holds no capability. The CLI bounds its reads: the module by `MAX_WORLD_MODULE_BYTES`, the fixture by `MAX_FIXTURE_BYTES`.

## Follow-ups

- Expose host events for recording, which needs a protocol decision about what joiners may read (out of scope here: no wire or journal change).
- Include the program snapshot at the window's start, so a partial window replays exactly.
- Add a start cursor to `record`, and let `complete` cover a window that starts at an install.
- Record the execution profile in the header, so a custom-profile world replays under its profile.
- Decide whether `ProgramInstalled.seed` should be withheld from joiners.
- Chat visibility for join-only participants (see the known gap above).

## Verification

Unit tests in `src/world_fixture.rs`: the round trip of every kind and of the null header variants; strict decode (unknown version, malformed lines, window errors); replay of the checked-in tic-tac-toe against the live instance's records; a partial fixture refused without `--allow-partial`; a reinstall window refused; a tampered header reported as `Mismatch`; a `territory` fixture that omits host events reported as `Mismatch`; the module check. An iroh test records a live world through `WorldClient` and replays it, and a second one records a trimmed history as incomplete. CLI parse tests cover `record` and `replay`.

For `verify` (F11.16, #544): unit tests cover `first_divergence` (identical sequences pass; the earliest step and first differing key are reported with both values; a key absent from one replay and an extra step are divergences), `verify` passes on the checked-in tic-tac-toe game, and a comparison-layer test double stands in for a deliberately nondeterministic guest — the sandbox gives guests no imports (no clock, no random source), so no checked-in guest can actually vary between instantiations; the double's records depend on a per-instantiation counter, the channel such a guest would use, and verify's comparison must catch it. A CLI parse test covers `verify`.

## References

- [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md)
- [Roles and directory record](../2026-10-17T08-00-00Z_feature_world_roles_directory/2026-10-17T08-00-00Z_feature_world_roles_directory.md)

---

## 2026-10-17T11-17-14Z Discovery: seed exposure verified; host-event mismatch demonstrated; implementation choices

- **Seed reaches joiners (verified, pre-existing).** A probe installed the checked-in tic-tac-toe with seed `424242` and read the stream as a join capability would. Both `events_after` and the snapshot's `events` serialize the install event with `"seed":424242`. The fixture leaves the seed out. It does not hide the seed from joiners, and the exposure predates this slice. Changing it touches the event serialization, so it is a follow-up, not part of this change.
- **Host events cause mismatches (demonstrated).** With the two `players` joins left out of the fixture, `territory` replays to `Mismatch` (test `host_events_missing_from_a_fixture_are_a_mismatch`). The records check catches this. Nothing in the fixture can repair it.
- **Install boundary, as implemented.** `replay_start` begins a replay at the last install. Chat and earlier installs may precede it; an input or a host event may not. A fresh world's full recording therefore replays, and a reinstalled world's window is refused.
- **Choices beyond the design.** `record` takes `--world` like the other world commands. `replay` refuses a fixture with no module hash, and the CLI runs `check_module` before compiling, because `replay` receives a compiled runner and cannot see the module's bytes. `read_history` gives up after 30 seconds (`READ_TIMEOUT`), and a recording retries its head three times (`HEAD_ATTEMPTS`). `MAX_FIXTURE_BYTES` is 128 MiB, which covers what a default world can record. `records` keeps its behaviour; `records_sequenced` is the consistent read that recording uses. The `record` arm runs `record_command` behind `Box::pin`, so `cmd_world` does not grow by the recording's futures.
- **Verification.** `cargo test --lib --no-default-features --features sandbox,world world_fixture`: 15 passed. The same with `test_cli_parse_world`: 10 passed, two of them new. `cargo test --lib` with default features: 1347 passed, 0 failed, 4 ignored (the ignored ones predate this change). `cargo fmt --check` is clean. `cargo check --lib --tests --no-default-features --features world` is clean. `cargo check --lib --no-default-features` fails with the same 12 errors as HEAD, in modules this change does not touch. Clippy, run with the rustup 1.97.0 cargo in its own target directory because the default cargo hits E0514, reports no errors. Its warnings match HEAD's apart from `cmd_world`'s estimated stack frame, which grows from 718,744 to 730,966 bytes; that lint predates this change, and splitting `cmd_world` is out of scope.
