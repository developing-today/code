# Design: sync, replication and FileOps

This chapter covers 39 design records: how ID nodes synchronize directories, events and journals; how replication is authorized and bounded; and the FileOps design that gives file operations one naming and ownership model. It describes the system as designed in the programme's design archive. The final section states what runs today.

The transport threads run under #5671 (transport fallback) and #5672 (storage decomposition), with children that include #5793 (rendezvous), #5800 (partial replication) and #5807 (Redis-to-SQLite export). The federation thread is #9106 Part B, with the discovery and registry design under #6287 and the federation cluster #9129–#9148.

## Directory sync and the first transport design

The earliest sync record is `2026-03-22T00-00-00Z_feature_peer-discovery` (2026-03-22): peer discovery via iroh-gossip plus a distributed topic tracker. `2026-10-10T02-39-03Z_design_directory_sync` (2026-10-10) designs directory sync proper: sessions, friend envelopes, and replication — the directory of [Design: identity, directory, groups and delegation](identity-directory-groups.md) moves between nodes inside signed friend envelopes over sync sessions. `2026-10-10T06-00-00Z_research_identity_capabilities` (2026-10-10), the research record on identity, capabilities, petnames and transport, is the shared research base for both chapters.

Two small records fix connection behaviour. `2026-10-11T11-08-42Z_feature_n0_relay_default` (2026-10-11) designs n0 relays by default: one endpoint policy and dial timeouts. `2026-10-11T04-48-13Z_feature_takeover_idle_connections` (2026-10-11) designs takeover passing idle keep-alive connections to the successor, so a restarted process inherits live connections instead of dropping them. Discovery is bounded and authenticated in `2026-11-02T08-13-35Z_design_peer-announcement-authentication` (2026-11-02): authenticated peer announcements and bounded discovery.

## The `/id-sync/1` split and the range-store seam

Two records from 2026-10-29 restructure the sync wire:

- `2026-10-29T16-21-00Z_design_id_sync_alpn_split` (2026-10-29) designs one `/id-sync/1` handshake, and records why `serve`'s range flights moved off the older arrangement: sync negotiation is one ALPN handshake, with range transfer as a method inside it.
- `2026-10-29T16-57-03Z_design_id_sync_range_store_seam` (2026-10-29) designs wiring `SyncMethod::Range` under the `/id-sync/1` handshake through a per-call lock store seam: each range call locks its store slice for the duration of the call.

The wire format decisions that complete the picture are `2026-11-16_decision_id5808_postcard_wire_schema_v1.md` (2026-11-16), the postcard wire schema v1 (envelope, discriminators and golden vectors), `2026-11-09_decision_id5810_carrier_framing.md` (2026-11-09), framing and fragmentation across ID carrier classes, `2026-11-09_decision_id5908_payload_boundary.md` (2026-11-09), the payload boundary decision procedure (decompose versus attach), and `2026-11-10_decision_id5809_versioning_negotiation.md` (2026-11-10), versioning, negotiation and forward-compatibility policy for the ID wire format. `2026-11-16T10-40-00Z_design_id5756_multiplexed_streams_seam.md` (2026-11-16) records the seam for multiplexed persistent streams over one tunnel (#5756).

## Journal replication, writer sets and partial replication

Journal replication is F12.13 under Phase 12 (#534). The design record is `2026-11-01_design_id806.md` (2026-11-01): journal replication as read replicas over a signed stream. The research package behind it is `2026-11-10T21-50-00Z_research_id806_d1-031-journal-replication` (2026-11-10, item D1-031). Two records handle the hard cases: `2026-11-03T14-55-00Z_design_id4630_oversized_replication_entries.md` (2026-11-03) designs replicating a journal whose `Checkpoint` entry exceeds the payload bound (#4630), and `2026-11-23T00-24-00Z_design_journal_ack_modes` (2026-11-23) designs selectable journal acknowledgement modes for world input. `2026-11-01_design_id2818.md` (2026-11-01) designs the journaled region-seam handoff for ID worlds, the point where a world's journal moves between regions; the residency meaning of a region is covered in [Design: residency, jurisdiction and boundaries](residency-jurisdiction.md). `2026-11-01T17-30-12Z_design_host_event_export` (2026-11-01) designs host-event journal export for world fixtures, so a recorded journal can be replayed as a test fixture.

Writer sets decide whose events a view includes. `2026-10-30T13-15-00Z_design_id_writer_set_view_projection` (2026-10-30) designs writer-set objects, view projection, and the discovery query (#5545's thread: writer-set view projection and discovery query). `2026-11-08T11-10-00Z_design_id_writer_sets_v2` (2026-11-08) designs writer sets v2: view-as-object, set algebra, federation, and discovery v2. The trust model that consumes writer sets (accept-all ingest, writer-set views) is in [Design: identity, directory, groups and delegation](identity-directory-groups.md).

Partial replication is designed in `2026-11-16T10-39-25Z_design_id5800_partial_replication_protocol` (2026-11-16): subset snapshot, scoped updates, existence privacy and dangling references (#5800). A node can hold a subset without learning what it does not hold, and references to unheld objects are a designed state, not an error. Replication is authorized, not open: `2026-11-03T02-34-54Z_security_world_replication_authorization` (2026-11-03) is the security record for world replication authorization.

## Relays, fallback and rendezvous

The fallback ladder is a designed sequence, each rung with its own record:

- `2026-11-01_design_id785.md` (2026-11-01) designs the UDP-denied SSH fallback for `id` (#785, Phase 13): when UDP is denied, sync falls back to an SSH-carried path.
- `2026-11-10_rendezvous_discovery_udp_denied_design.md` (2026-11-10) decides rendezvous and discovery for UDP-denied clients (#5793's thread).
- `2026-11-10T19-55-00Z_design_id5796_connect_proxy_traversal.md` (2026-11-10) designs HTTP CONNECT proxy traversal for ID.
- `2026-11-05T13-38-00Z_design_id5791_transport_negotiation.md` (2026-11-05) designs transport negotiation and fallback ordering: the ladder has a stated order, negotiated rather than tried at random.
- `2026-11-08T11-08-53Z_design_id5813_relay_proxy_protocol` (2026-11-08) designs the ID relay and proxy protocol, and `2026-11-14T13-20-00Z_design_id5795_relay_trust` (2026-11-14) decides the relay trust model and broker authorization (#5795).
- `2026-10-29T11-05-00Z_investigation_multi_hop_relay_and_p2p_media` (2026-10-29) investigates multi-hop relay and peer-to-peer media paths.
- `2026-11-01_design_id2036.md` (2026-11-01) designs the ID messaging and relay protocol v1 (#2036, a sub-epic of #2056 and one of its two open items at charter time).

Discovery and federation close the section: `2026-11-16_design_id6287_discovery_registry_federation_v1.md` (2026-11-16) designs discovery, registry and federation v1 (#6287), and `2026-11-08T09-10-14Z_design_id5899_provider_discovery_negotiation` (2026-11-08) designs provider discovery and negotiation protocols (#5899), cross-linked from [Design: capabilities and providers](capabilities-providers.md).

## FileOps: naming, ownership and save reservations

FileOps is the design that gives every file operation one implementation and one naming model. The design lineage is tracked as #4766, with the audit under #4767. The full record-by-record text of the #4766 lineage could not be re-read for this chapter (the laptop checkout was unreachable during drafting); the naming and ownership rules below are stated from the shipped implementation as recorded in the web and configuration survey, and the #4766/#4767 correspondence is marked unverified beyond the commission's own statement of it.

The rules as implemented, which the design fixes:

- A file is a name pointing at an immutable blob hash, plus tag-store metadata. The name is an iroh-blobs tag; the bytes never change under a name — a new write is a new blob and a re-pointed name.
- Rename and copy preserve history as archives. The archive name is `{name}.archive.{unix-ts}`, with a literal example: a file named `notes.txt` renamed at Unix time 1700000000 leaves an archive named `notes.txt.archive.1700000000`. On a same-second collision the timestamp suffix is bumped by 1 until the name is free, claimed under an in-process lock (#1612/#2462).
- The archive namespace is reserved at the writer: a user file name matching `{stem}.archive.{digits}`, where the digits must fit in a `u64`, is refused with `ReservedArchiveName`. A non-numeric look-alike such as `a.archive.notes` stays a valid user file name.
- Ownership is explicit metadata. The keys are `archive.rename` and `archive.replace` on the live name (naming the archives it owns) and `archive.owner` on the archive (naming its owner). Archives are dropped only with their owner; there is no per-archive delete. The identity keys `name`, `file` and `path` are refreshed on rename and copy, and the `origin` key is stamped at first write (#4973) — the shipped form of the listing-provenance design in [Design: foundations — objects, graph and the file core](foundations.md).
- Delete is a hard delete of name, metadata and owned archives; blob bytes survive until garbage collection. The web layer's soft-delete and restore pair is built on top of FileOps, not inside it.

Concurrent saves are handled in the design by `2026-11-02T07-00-00Z_design_save_reservations` (2026-11-02): save reservations give in-flight exclusion for same-file saves, so two writers of one file serialize by reservation instead of racing the store.

Three records surveyed into this chapter belong to adjacent surfaces and are cross-linked, not re-argued: `2026-11-08T05-48-47Z_design_input_event_model` (2026-11-08, the input event model for relayed apps), `2026-11-08_design_id5862.md` (2026-11-08, board subscription, discovery and feed integration), `2026-10-30T19-45-00Z_investigation_playable_posts` (2026-10-30, playable posts with forkable succession saves), and `2026-11-12_design_id9004.md` (2026-11-12, Joplin continuous sync as a target/plugin bridge against the #5848 baseline). The multi-device knowledge-layer sync design, `2026-11-09_design_id5838.md` (2026-11-09) — local-first sync, conflicts, offline behaviour, end-to-end encryption and attachments — is the knowledge layer's consumer of this chapter's machinery.

## The walk skeleton as the design's first proof

Epic #9106's design is demonstrated, not only described. The demonstration path is the walking skeleton: the record `2026-11-22T16-13-20Z_design_id-walking-skeleton` is surveyed into the roadmap chapter, and the shipped form is the `id walk` command family. The skeleton walks the whole stack in miniature: a node directory holds signed records, writers append with named parents, two writers produce a conflict, heads and conflict reports are computed, and two nodes sync. It is the first proof that the #9106 primitives — content-addressed records, signed authorship, head tracking, conflict reporting and peer sync — compose into a working path. The design chapters treat it as the reference instance of the sync design: where a design rule in this chapter has a walk counterpart, the walk counterpart is the test of the rule.

## What exists today

Grounded in the code surveys, not the design records:

- FileOps ships as a single implementation: `crates/id-sync/src/fileops.rs` is the one put, rename, copy and delete implementation used by the CLI, the REPL, the meta protocol and the web UI, with the archive naming and ownership rules stated above, verified against the code in the web survey.
- The walk skeleton ships and is not feature-gated: `id walk` provides `init`, `write`, `read`, `reports`, `serve` and `sync`. Writers in the skeleton are demo keys derived from the writer name (`--as` accepts the skeleton's toy writer names); reading an address the node does not hold is an error, not a fetch; one process may hold a node directory at a time.
- The federation record store ships in `crates/id-sync` with named slices covering key lifecycle and revocation, replay guard, signature suites, signer binding, rejection events, conflict reports, sync scope, anti-entropy, peer priority, fanout budget, query grants and privacy, signed receipts, request identity, result dedup, loop inform, routing state, topology probe, takeover response, peer classes and the ingest gate. The envelope ALPN is `/id-envelope/1`.
- Ingest outcomes in the shipped event store are accepted, duplicate, fork, waiting and refused — the same five outcomes the hosted bridge returns, by design.
- Still design only, with no shipped counterpart verified in the surveys: the full partial-replication protocol of #5800 (subset snapshot with existence privacy) and the multi-hop relay path as a general facility. Their record-level design is complete in the archive; their shipped status is unverified in the survey material.
