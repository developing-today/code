# D1-031 — F12.13 journal replication to signed read replicas: research package

**Short answer:** F12.13 replicates the authoritative single-writer world
journal — not the records projection — over a dedicated signed stream
(ALPN `/id-world-replication/1`). A replica verifies every frame against a
pinned source identity before persisting it, restores through the existing
world restore path in a presence-preserving, read-only mode, and serves
spectator sessions from its last verified cursor while the writer is
offline. Tampered, gapped, forked, replayed, or unsupported-version frames
are refused with the last complete replica left intact. The decision-ready
design is `doc/2026-11-01_design_id806.md`; implementation phases 1–4 have
since landed under issue #806 (PRs #4447, #4589, #4634, #4708, all merged
2026-11-02/03). Phase 5 (operator interface and resilience) and the
offline-admission decision remain open on #806; neither is research scope.

- Tracker: issue #536, item D1-031 (XL, research package)
- Implementation issue: #806
- Design: `doc/2026-11-01_design_id806.md` (decision-ready, 2026-11-01)
- Source: `.opencode/plans/phase-12-platform-flexibility.md:55`
- Package date: 2026-11-10; current-main evidence verified at
  `9b65fa2ec8` (merge of PR #8311)

## The question

From the plan (F12.13): "Read replicas that follow a world's journal over
a signed stream. A world keeps a single writer, so there are no write
conflicts. Replicas serve spectators and survive the writer being
offline. The rollback and format rules from F11.12 apply to the stream.
Exit: a replica reproduces the writer's records, and a tampered stream is
refused."

The tracker item restates it: read replicas follow a single-writer world
journal over a signed stream, serve spectators and survive writer
downtime; tampered streams are refused; the F11.12 format rules apply.

## Baseline when the tracker item was written

The status text on #536 predates the implementation: the `Mirror` and
`Download` world commands copy modules and records, and no journal-follow
stream existed (`grep -rn 'replica' pkgs/id/src/world*.rs` found nothing).
The existing `world_records` replication is a cache of the current
key/value projection — it coalesces changes, omits history, and cannot
restore or serve a world (`pkgs/id/src/world_records.rs:1-18`). That
projection is therefore not a replication source of truth; the journal
is. This baseline is recorded here because it is the gap the research
package had to close; it is no longer the current state (see Evidence).

## Options considered

Summarized from design §3 (`doc/2026-11-01_design_id806.md`):

| Option                                                                        | Why not / why                                                                                                                                                                            |
| ----------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Extend the `world_records` iroh-docs document to carry journal data           | A CRDT key/value document is the wrong authority for an ordered journal: it represents the latest projection, coalesces intermediate changes, and cannot directly restore/serve a world. |
| Send the journal through the existing interactive `WORLD_ALPN` session        | Couples replication lifecycle/versioning to participant frames; no replication cursor, durable change feed, checkpoint-reset contract, or replica read-only mode exists there.           |
| Re-send a complete signed journal snapshot on every sync                      | Smallest protocol, but repeats all journal and module bytes on every reconnect. Retained as the initial-sync and compaction-recovery mechanism, not the steady-state path.               |
| **Dedicated signed replication stream with snapshot/reset plus ordered tail** | **Chosen.** Keeps the authoritative JSONL journal, replication integrity, and participant sessions separate; supports reconnect, compaction, and offline serving.                        |

## The contract (design §4, as implemented)

1. **Source and publication.** The source is the existing single writer.
   It emits a snapshot at first sync, then ordered durable journal-entry
   batches. A frame is published only after `WorldJournal::append`
   succeeds (fsync-before-acknowledge); compaction or a journal-header
   version bump emits a complete signed reset snapshot before later tail
   entries. The actor/store owns the feed, so snapshot-plus-subscribe has
   no race and never reads a journal inode while compaction replaces it.
2. **Cursor.** `(entry ordinal, rolling digest)` over the exact
   `JournalEntry` bytes, seeded with the replication domain, world ID,
   source node ID, and current header version; recomputable from the
   JSONL file after restart. A replica whose cursor is not in the
   current chain receives a full signed reset snapshot. Every signature
   binds a fresh random session nonce, so captured frames cannot be
   replayed into another sync session.
3. **Signed envelope.** Carries world ID, source node ID, journal header
   version, transfer kind, stream cursor, previous-frame digest, entry
   range, and payload digest; signatures bind a replication-specific
   domain tag and the exact serialized envelope bytes, verified with
   Ed25519 `verify_strict`. Only the explicitly selected/pinned source
   node ID is trusted, and the signer must match it — a key supplied
   only by the stream is never accepted.
4. **Payload.** The exact serialized `JournalEntry` bytes (including
   JSONL entry boundaries), never a lossy projection. Frames are
   size-bounded; larger module payloads are chunked and reassembled
   against a declared hash. The stream carries the active module bytes
   and checks them against the journal's BLAKE3 module hash. A signed
   terminal frame commits the completed snapshot/batch digest and
   cursor; incomplete transfers do not replace current replica state.
5. **Receiver.** Checks signature, trusted signer, domain, world ID,
   cursor continuity, previous digest, entry decoding, header version,
   and module hashes before making a batch visible. Gaps, conflicting
   repeats, malformed/tampered frames, and unsupported journal versions
   are refused. A complete reset is staged beside the active replica
   and atomically installed; a tail is durably appended only after its
   complete signed batch verifies.
6. **Framing ownership.** The design gated the outer envelope on F12.6's
   framing contract (#1063 slice 1), with an explicit fallback: if
   F12.6 defines no reusable wire framing, F12.13 owns its versioned
   envelope. The landed wire is that dedicated envelope on its own ALPN
   (`/id-world-replication/1`); OTLP export was not made a prerequisite.
   The later extension record
   `doc/2026-11-03T14-55-00Z_design_id4630_oversized_replication_entries.md`
   extends this §4 contract without changing the wire.

## Replica lifecycle

- **Presence-preserving restore.** Ordinary restore synthesizes
  presence-off events for participants still present at restore time.
  That is correct for a writer restart and wrong for a replica: replica
  restore restores presence at the snapshot cursor as session state
  without feeding synthesized presence events into the program, and no
  synthetic event is journaled or counted against the verified cursor.
  Phase 4 acceptance includes a snapshot taken with participants
  connected, proving the replica's `Records` equal the writer's at the
  same cursor.
- **Read-only service.** The replica reuses `world_store::open_world`
  reconstruction but through a non-mutating open path (no writer-side
  `FileJournal::open` recovery/header-bump behavior). It exposes only
  spectator/read-only access: no world mutations, admin operations,
  local schedule/tick firing, or program capability side effects.
  Viewer presence is never appended to the replicated journal.
- **Directory facts are in scope.** Spectator authorization consults
  directory facts (`WorldCore::effective_scopes` intersects account
  ceilings and zeroes scopes for cross-account delegations whose
  accounts are not friends), so the signed snapshot carries the account
  ceilings and friendship graph for accounts appearing in capability
  chains, bound into the snapshot digest. A journal-only replica would
  wrongly reject a valid friend-delegated spectator capability.
- **Offline serving.** After sync, the replica serves the existing
  participant protocol from its local `WorldService`; spectators do not
  need the writer online. The replica exposes its last verified
  cursor/time as staleness metadata and never claims to be fresher.
  The records-only `id world mirror` command keeps its semantics; the
  journal replica is a separate operation.

## Invariants (design §5)

1. One writer; replicas never commit a journal entry or originate
   authoritative world mutations.
2. The replicated payload preserves journal order, entry bytes,
   `Created` header/version, and checkpoint semantics; no stream may
   reorder, omit, or synthesize entries.
3. Acknowledged source writes are streamable only after durable append;
   a source crash or interrupted transfer leaves the last complete
   replica usable.
4. Every accepted frame is bound to the selected source identity,
   world, session nonce, domain, cursor, and preceding digest;
   tampering, truncation presented as complete, replay at an
   incompatible cursor, a gap, or a fork is refused.
5. F11.12 versioning is preserved: a replica refuses a journal version
   newer than it supports at the header, before applying any entry;
   header bumps/reset snapshots precede entries needing the newer
   format.
6. A replica reproduces the writer's program `Records` at the same
   verified journal cursor, including after restart and checkpoint
   compaction; `Records` remain a derived projection, not the source
   of truth.
7. A replica serves spectator sessions while disconnected, reports its
   verified cursor, and never claims to be fresher than that cursor.

## Evidence — what has landed

Implementation phases from design §7, verified on GitHub 2026-11-10:

| Phase | Scope                                          | PR    | State                                                 |
| ----- | ---------------------------------------------- | ----- | ----------------------------------------------------- |
| 1     | Replication wire contract + crypto primitives  | #4447 | MERGED 2026-11-02                                     |
| 2     | Serve signed world journal replication         | #4589 | MERGED 2026-11-02                                     |
| 3     | Durable replication receiver + verifier        | #4634 | MERGED 2026-11-03                                     |
| 4     | Read-only replica world service and spectators | #4708 | MERGED 2026-11-03                                     |
| 5     | Operator interface and resilience              | —     | No merged PR in this evidence set; remains #806 scope |

Current-main module map, verified at `9b65fa2ec8` on 2026-11-10:

- `pkgs/id/src/world/replication.rs` — wire contract and crypto
  primitives (phase 1).
- `pkgs/id/src/world_replication_net.rs` — signed replication streams
  for world writers; `REPLICATION_ALPN = /id-world-replication/1`;
  registered as `pub mod world_replication_net` in
  `pkgs/id/src/lib.rs` (phase 2).
- `pkgs/id/src/world/replication_receiver.rs` — durable receiver and
  verifier (phase 3).
- `pkgs/id/src/world/replica.rs` — read-only replica world service
  (phase 4); serving wired through `pkgs/id/src/commands/serve.rs`,
  `pkgs/id/src/world/actor.rs`, and `pkgs/id/src/world/handle.rs`.

Acceptance mapping to the plan exit criteria: "a replica reproduces the
writer's records" is the phase 4 acceptance scope (#4708, including the
presence-preserving snapshot case); "a tampered stream is refused" is
the phase 3 negative-test scope (#4634: tamper, truncation, gap, fork,
unsupported version, size limits). This package cites those merged PRs
as the evidence; it does not re-run their test suites.

## Prerequisites, reconciled

- #227 — D1-012 F11.12 journal format version header, check and backup
  commands: **CLOSED**. The design records F11.12 as landed (journal
  versions 1 and 2 supported; the writer bumps the header atomically
  before appending an incompatible entry; restore refuses an
  unsupported header before applying entries).
- #534 — the Phase 12 research-workflow umbrella: **CLOSED**. Note the
  gap this package closes: #534's committed Phase 12 package
  (`doc/2026-11-03T00-05-09Z_research_phase-12-package-534/`) inventories
  11 D1 items (D1-019, D1-021–D1-028, D1-032, D1-035) — D1-031 belongs
  to tracker #536, not #534, and had no concept in that package. The
  design and implementation existed; the research-package artifact for
  the tracker item did not. This document is that artifact.

## Open decision carried, not decided

Design §8: admission of new spectators after a writer outage. A
disconnected replica cannot learn of a revocation made after its last
verified cursor, so this is a security/availability promise, not an
implementation detail:

- **A** — Continue indefinitely from the last verified journal (best
  availability; a recently revoked spectator may retain access until
  reconnect; stale cursor must be shown).
- **B** — Require a writer-signed admission lease; after expiry, refuse
  new spectator sessions. **Design recommendation: B**, lease duration
  configurable per replica with a conservative default, because
  journaled revocation is an access-control boundary.
- **C** — Never admit new sessions while disconnected (strongest
  freshness; does not meet the "replicas serve spectators while the
  writer is offline" requirement). Not recommended.

This decision is Drew's and stays on #806. Regardless of the choice, a
replica never grants more than spectator scope and applies every
revocation present in its last verified journal.

## Non-goals

Multi-writer replication, conflict resolution, and failover promoting a
replica to writer; replicating the records projection as the authority
or changing the records-only mirror; replicating unrelated world stores
(petnames, outbox); OTLP export itself (F12.6 work).

## Tracker consequence

With this package committed and implementation issue #806 linked from
#536, D1-031 meets the tracker's own criterion — "the design (or
research package) is committed and an implementation issue is linked"
— on the strength of the committed artifact, not of this package's
claims about the still-open phase 5 or the §8 decision.
