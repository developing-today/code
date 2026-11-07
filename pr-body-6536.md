## Phase 3 of #806 (F12.13): durable receiver + verifier

Implements phase 3 of the phased plan in `doc/2026-11-01_design_id806.md` §7, on top of phase 1 (PR #4447, wire contract + `ReplicationTracker`) and phase 2 (PR #4589, writer snapshot-plus-subscribe feed; merged as 51535c2b, this branch's base).

### What lands

New module `pkgs/id/src/world/replication_receiver.rs`: `DurableReceiver`, a durable replica of one world's journal from one pinned source.

- **Pinned-source verification** — every frame goes through the phase-1 `ReplicationTracker` built with the receiver's pinned source; untrusted signers, bad signatures, nonce replays, gaps, forks, stale replays, unsupported journal versions, and oversized payloads are refused by the tracker before payload staging.
- **Staged transfers** — accepted snapshot/tail payloads are staged in memory only. Nothing becomes durable until the batch's signed terminal frame verifies (§4.4/§4.5): truncated, refused, or disconnect-abandoned transfers leave the last committed replica untouched. A staged-transfer size bound (64 MiB) keeps an unterminated stream from growing memory without limit; exceeding it poisons the session rather than desynchronising staging from verification.
- **Atomic snapshot install, append-only tails** — a committed snapshot is written to a scratch file, fsynced, renamed over `journal.jsonl`, and the directory is fsynced, so a crash mid-install never leaves a torn journal (a leftover scratch file is cleaned up at open). Committed tail batches append + fsync; committed bytes are never rewritten.
- **Restart / reconnect** — `DurableReceiver::open` re-reads the committed journal, refuses it if it is not a well-formed chain for this world at a supported header version (decodable lines, exactly one leading `Created` header), and recomputes the verified cursor from the entry bytes under the pinned source. Every session opens with a snapshot (the phase-2 feed contract), which rebases the tracker; staged bytes from a dead session are discarded.

### Scope note — module transfer

The packet scope mentions module transfer/hash checks. The phase-1/2 contract as built carries journal bytes only — there is no module-bytes frame kind in v1, so there is no module payload for the receiver to hash-check. Module hashes named by journal entries (e.g. `BuilderInstalled.module_hash`) are covered by the cursor chain like every other entry byte. This is recorded in the module docs; adding a module frame kind is a wire-contract change for a later phase, not something phase 3 can invent receiver-side.

### Tests

11 tests in the module (laptop, `cargo test --lib replication_receiver`): snapshot commits only on terminal; truncated/abandoned transfer commits nothing; tail batch withholds append until its terminal, then appends exactly; tampered payload refused with committed replica intact; untrusted source refused; gap and fork tails refused; unsupported journal version refused before any entry; oversized payload refused; captured frames replayed into a new-nonce session refused; reset snapshot replaces the journal and bumps the header version; restart reopens committed state and a corrupt journal is refused at open; frames without a session are refused.

Refs #806 (phase 3 of 5; #806 closes when phase 5 lands). Queue task 6536.
