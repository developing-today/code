# Events and the signed audit chain

Almost everything durable in ID is an event. Files have names and blobs, worlds have journals, and groups have membership, but the record that ties them together — who did what, in what order, under whose authority — is the signed event store. This chapter describes the store as it ships: its on-disk format, its wire format, the vocabulary of event kinds, and the surfaces built on top of it. The neighbouring chapters cover the file core and sync; this one covers the log itself.

## The store on disk

The event store lives in a single redb file named `events.redb` in the store directory (a temporary sibling, `events.redb.tmp`, is used during writes). Its format marker, recorded in the `meta` table, is `id.events.store.v9`. The previous marker, `id.events.store.v8`, is also accepted: a v9 store differs from a v8 store only in that it may carry a `bodies` table.

The v9 store has 17 tables: `events`, `hashes`, `chains`, `forks`, `groups` (a multimap), `proof_ids`, `proof_headers`, `waiting`, `waiting_by_dep` (a multimap), `meta`, `blocks`, `writer_set_bodies`, `actions`, `device_ops`, `applied`, `bodies`, and `tag_fold`. Keys are big-endian, so one author's events sort in sequence order inside the tables. A chain row is a fixed 41-byte encoding: the next sequence number as a big-endian `u64`, the 32-byte head hash, and a frozen flag byte (0 or 1).

The implementation sits in `id-events` (`crates/id-events/src/event_store/`), behind the `Storage` / `FileStorage` seam. The event store's own vocabulary types — the op vocabularies the signed header embeds, such as chat, notes, note links, session checkpoints, issuer rotation and policy — live one layer down in `id-store`, with the header type that carries them. The operator block list is stored in the event store's `blocks` table; it is data, not configuration.

## The wire format: canonical-CBOR v1 headers

Every event on the wire is a p2panda-core header, `Header<ChainExt>`, encoded in canonical CBOR. The domain tag is `id.events.v1`. The operation ID of an event is the BLAKE3 hash of the received bytes. Signatures are checked with `verify_strict`: an event whose signature does not verify is not admitted.

The extension carried in each header, `ChainExt`, has three fields: `kind`, `group`, and `after`. The `kind` is 1 to 64 bytes and is normalized to NFC. The `after` field names up to 32 prior hashes (`MAX_AFTER` = 32); it is the dependency list that lets a receiver order an event against the events it builds on, and hold it in the waiting pool when a dependency has not arrived yet.

Ingest has five outcomes, and the same five outcomes appear in the hosted bridge described in [The hosted edition](hosted.md): accepted, duplicate, fork, waiting, and refused. A fork is evidence, not an error to be hidden: the `forks` table keeps it, and a forked chain is how double-signing by one author becomes visible to everyone who syncs the log.

## The event-kind vocabulary

Event kinds are string constants. The reference inventory names 23 of them across `id-store`, `id-events` and `id-core`, plus the group-operation kinds carried by `id_events::groups`. The named set includes:

- Chat and documents: `chat.message.v1`, `chat.anchor.v1`, `document.put`, `document.remove`, `media.put`, `media.remove`.
- Notes: `note.create`, `note.revise`, `note.link`.
- Tags: `tag.set`, `tag.clear`.
- Sessions: `session.checkpoint`.
- Identity and devices: `identity` (the documents/identity kind), `issuer.rotation`, `device.add`, `device.revoke`.
- Attestation: `attestation.genesis`, `attestation.claim`, `attestation.witness_decision`, `attestation.objection`.
- Writer sets: `idset.writer`, `idset.rescind`.
- Chained messages carried by `id-sync`: `chained.message`.

Group operations — group create, member and role changes — are carried by the group kinds owned by `id_events::groups` and validated during ingest and resolution in the event store. The exact spelling of every group-operation kind was not enumerated in the surveys and is unverified in this chapter; the reference chapter on data stores carries the full table.

## Documents, claims and writer sets

Three surfaces sit directly on the event store.

Documents are events of kind `document.put` and `document.remove`, addressed through `DocumentRef` in `id-store`, with `DOCUMENT_KIND` naming the document kind. A document is therefore not a separate database: it is a projection of the event log, rebuilt by folding the log.

Claims are the local-claims surface in `id-events` (`local_claims`), with the tag-claims hash domain `id.local-claim.v1` in `id-store`. A claim is a signed statement by one author about a subject; the fold decides what a given reader's view shows.

Writer sets decide whose events count for a given view. The types live in `id-events` as `idset::{WriterSet, Entry, Excludes}`, with the view expansion in `idset::view::expand_set` and the set bodies stored in the event store's `writer_set_bodies` table. Events by authors outside the writer set are still stored and synced; the writer set governs projection, not admission. That distinction — accept-all ingest, writer-set views — is the trust model of the whole system.

## Journals and checkpoints

Two other logs reuse the same ideas at smaller scale.

The directory keeps an append-only JSONL journal, `directory.jsonl`, holding one `DirectoryEntry` per line across 17 entry kinds (account sign-up, key add and remove, group create and update, member and tier changes, friend requests and removals, and the rest). The journal is replayed through the same `apply` path as live changes, so a replayed directory and a live directory reach the same state by construction.

Worlds keep their own journals. A world host checkpoints its journal on a fixed cadence: `id serve` takes `--world-checkpoint-every`, default `1000`, where `0` means never trim. Note editing has its own checkpoint kind, `session.checkpoint`, and the collaborative editor can publish a durable note-session checkpoint (collab tag 10), answered by tag 11 carrying the hex of the new note head revision; filename sessions refuse that frame. Session checkpoints began as a proof of concept (#2241) and a design for a session header plus segment checkpoint event kind (#2240); the shipped kind constant is `session.checkpoint`.

## The audit chain

The audit chain is the event chain itself, read as evidence. Each author's events form a hash chain: every header commits to its position by sequence and by the `after` dependencies, the chain row in the store records the head hash and next sequence, and each header is signed by its author. To audit is to re-verify: recompute the BLAKE3 operation IDs, check every signature with `verify_strict`, confirm the sequence has no gaps, and confirm that no fork exists for the range in question — or, if one does, that it is recorded in `forks` and attributed to the author who double-signed.

The same pattern recurs wherever ID needs a tamper-evident record. The MCP policy point, described in [Agents in worlds: the MCP bridge and policy](mcp-and-agents.md), writes exactly one hash-chained, journal-key-signed audit event per agent action, refusals included. The hosted bridge store mirrors the native tables under `bridge_` names (`bridge_events`, `bridge_chains`, `bridge_forks`, `bridge_proof_ids`, `bridge_proof_headers`, `bridge_waiting`, `bridge_waiting_by_dep`, `bridge_meta`), so a hosted peer holds the same evidence in the same shape.

One honesty note on scope: the surveys establish the store format, the wire format and the kind vocabulary from the code at the surveyed commit. They do not publish a worked example of a full chain verification, and this chapter does not invent one. A toy illustration, with invented data only: author key `aa…` (a placeholder name, not a real key) signs event sequence 0 with kind `note.create`; sequence 1 names sequence 0's hash in `after`; a verifier who receives sequence 1 first holds it as waiting until sequence 0 arrives, then accepts both in order.

Next: [Residency, jurisdiction and boundaries](residency.md) covers where events are allowed to live; [The hosted edition](hosted.md) covers the peer that holds them for hosted users.
