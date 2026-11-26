# Design: foundations — objects, graph and the file core

This chapter covers the foundation layer of the ID design archive: 32 of the 408 ID-programme design records classified in the book survey. It describes the system as designed in those records. A designed behaviour in this chapter is a statement about the design archive, not a statement that the behaviour runs today; the final section separates the two.

Two epics frame the chapter. Epic #2056 is the core ID programme epic: most design records in the archive state `Programme: epic #2056` in their headers, and the programme charter record (`2026-11-23T12-06-42Z_design_id_program_investigation_prompt`, 2026-11-23) defines ID as a local-first, federated, capability-based application platform. Epic #9106 covers extensible primitives, federation and namespaces; its Part A is the core this chapter describes, Part B is federation and trust, and Part C is the Atlas-facing work tracked as #9109. The store-port contract thread under #9106 is #9110, with slices #10191 and #10213 and the adoption call #9082.

## The object model and the signed event graph

The foundation design puts a signed event graph under every other subsystem. The records treat graph objects as the unit of storage and exchange: content-addressed blobs hold bytes, and signed events record who asserted what, in which order, under which authority. Three records fix the invariants the rest of the programme builds on:

- `2026-11-01_design_id2898.md` (2026-11-01) states the admission rule in its title: reconstruction is admission. A node admits an object by reconstructing and verifying it, not by trusting the peer that delivered it. The same rule is homed in the capability chapter's record set because admission is also a capability boundary; see [Design: capabilities and providers](capabilities-providers.md).
- `2026-11-01_design_id3231.md` (2026-11-01) designs one receipt contract for `pkgs/id` artifacts (#3231), and `2026-11-04_audit_id3232_receipt_validation.md` (2026-11-04) is the Phase 3 audit of receipt validation under #3232. The design intent is a single receipt format across artifact types instead of one format per subsystem.
- `2026-11-17_design_id_sign_time_refusal.md` (2026-11-17) states the signing-side complement: sign only what receipt would accept. Signing and receipt validation are designed as two ends of one contract.

The graph also carries session structure. `2026-11-01_design_id2240.md` (2026-11-01) designs a session header plus a segment checkpoint event kind, so a long event stream has named segments and checkpoint points instead of one undifferentiated chain. `2026-11-01_design_id805.md` (2026-11-01) designs delta and binary frames (F12.11), the frame-level encoding for moving graph updates between nodes.

Retrieval of large graph objects gets its own investigation: `2026-10-30T19-28-00Z_investigation_bittorrent_chunk_retrieval_for_graph_objects` (2026-10-30) examines BitTorrent-style chunk retrieval for graph objects (archive list item 14b). The record is an investigation, not a decision; the chapter records it as an open design direction, unverified as adopted.

## Addressing: `id://`, friendly names and related paths

Three naming layers sit on top of content addressing in the design:

1. Content hashes name immutable bytes.
2. The `id://` URI scheme names objects and namespaces in a portable text form. `2026-11-17T04-55-00Z_design_id9117_id_uri_scheme.md` (2026-11-17) is the design record for the scheme and its per-namespace handlers and encodings (#9117).
3. Friendly names give people local, human-readable references. The lineage is three records deep: `2026-10-30T13-25-00Z_design_friendly_name_references.md` (2026-10-30) specifies local-store friendly-name reference resolution as the proof of concept for #2259; `2026-11-01_design_id2259.md` (2026-11-01) is the #2259 design itself; `2026-11-04T15-33-00Z_design_friendly_name_fanout.md` (2026-11-04) extends resolution to cross-store fan-out as a first slice (v2).

`2026-11-17T22-00-00Z_design_id9114_related_path_links.md` (2026-11-17) designs related path links carried in `hash.metadata` (#9114, sections S3.21–S3.22 of its parent design), so a hash can point at related paths without a separate index. `2026-11-17_design_id9127_append_list_user_attributes.md` (2026-11-17) and `2026-11-23_design_id9128_social_app_reference_model.md` (2026-11-23) extend the object model at the edges: append-list user attributes (#9127) and a social-app reference model (#9128).

Atlas enters the foundation design at two points. `2026-11-09T07-20-00Z_design_atlas_id_publication` (2026-11-09) designs the Atlas-to-ID-graph publication format: corpus objects plus provenance and lineage relations published as ID-graph objects. `2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces` (2026-11-13) is the proto-design for ID/Atlas extensible primitives, federation and namespaces, and `2026-11-13T03-40-00Z_design_id-atlas-summit` (2026-11-13, a 104-file bundle) records the summit decisions, defaults and owner questions across its parts A, B and C. These records are the foundation end of the #9106 Part C / #9109 thread.

## The file core and its design lineage

ID began as a file-sharing system, and the foundation records keep the file core as a first-class layer rather than a legacy residue. The original architecture, preserved in `pkgs/id/ARCHITECTURE.md`, is a CLI, REPL and web UI over protocol, store, tag store and discovery layers, over Iroh blobs, docs and gossip, with an alpha/omega dual-index tag system. The design records then extend that core:

- `2026-11-01_design_id2146.md` (2026-11-01) designs programs as websites in `id serve`, turning the file-serving core into a program host.
- `2026-11-02T11-47-29Z_design_web_listeners` (2026-11-02) designs web listener bindings for `id serve`.
- `2026-11-04T21-50-26Z_feature_dynamic_publications_runtime` (2026-11-04) designs the dynamic publication runtime apply path and its operator CLI.
- `2026-11-04T10-46-17Z_design_listing_origin_and_upload_time` (2026-11-04) adds file listing provenance: origin node and upload time in list output. The origin stamp is the design ancestor of the `origin` key that FileOps writes at first write (#4973); the FileOps design itself is covered in [Design: sync, replication and FileOps](sync-fileops.md).
- `2026-10-29T12-50-36Z_design_id_app_packaging` (2026-10-29) designs ID app packaging: an app manifest, a folder layout, and TUI/web parity as a packaging requirement.
- `2026-11-07T04-33-34Z_architecture_secure_public_web_routers` (2026-11-07) designs secure public web router constructors, the boundary between the local file UI and publicly served routes.

Two records fix housekeeping rules for the whole object space. `2026-11-08T10-28-00Z_design_id6009_public_domain_normalization` (2026-11-08) normalizes public-domain storage to CC0-1.0 for content and 0BSD for code. `2026-11-08T09-12-27Z_design_id-self-hosting-interop-stance` (2026-11-08) records the self-hosting and interoperability stance. `2026-11-08T23-17-06Z_design_id_setup_ceremony` (2026-11-08) designs the native ID creation and setup ceremony, the first-run path that creates an identity and its stores. `2026-11-04T05-11-11Z_feature_structured_logging` (2026-11-04) designs structured operational logging for the whole binary.

The payload grammar has its own record: `2026-11-16_decision_id5753_kdl_fork_section5.md` (2026-11-16) decides the KDL fork design pass for #5753 section 5 — payload grammar, extension marking and serde form. `2026-11-09_audit_3068-phase3-closing.md` (2026-11-09) is the Phase 3 closing audit of the parsed-form contract (#3068 / #3069). `2026-11-08T13-20-00Z_research_id6014_ocapn_captp_watch` (2026-11-08) is a watch record on OCapN / CapTP stability and the CapTP bridge spike (#6014): the foundation design tracks that protocol family as a candidate bridge, without adopting it. `2026-11-13T10-15-00Z_design_scaling_technique_comparisons` (2026-11-13) compares scaling techniques at the foundation layer; the performance programme that owns the measurements is covered in the scale chapter of this book.

## Structure: the crate split and store ports

The foundation design ends at code structure. `2026-10-17T15-00-00Z_architecture_next_stage_platform` (2026-10-17), "Next stage of the platform: foundations, then flexibility", sequences the work: foundations first, flexibility second. Phase 12 (#534) executes the flexibility half, and Phase 13 executes the client-surface half.

The crate split has two records: `2026-11-01T15-42-00Z_architecture_id_crate_split` (2026-11-01, Phase 1 architecture) and `2026-11-01T16-53-00Z_design_id_crate_split` (2026-11-01, a workspace of importable crates). The design result, recorded in the appended half of `pkgs/id/ARCHITECTURE.md`, is a workspace in which only the root `id` crate is the public surface, `id-core` is the shared core depended on by 10 crates, and the blob, event, world and sync ports are distinct authorities: adapters serve exactly one port, and the rule is enforced by `tests/store_port_boundary.rs`. That anti-flattening rule is the structural form of the foundation design: no subsystem reaches into another subsystem's store.

One record in this chapter is present by classification, not by subject fit: `2026-11-01_design_id560.md` (2026-11-01) designs the F12.18 denial-of-service suite in CI. The survey homes it here; its substance belongs to the scale and hardening programme. `2026-11-11T03-19-54Z_refactor_markdown_module_split` (2026-11-11) is a refactor record for the split of `pkgs/id/src/web/markdown.rs` (#4704), retained here as file-core history.

## What exists today

Grounded in the code surveys, not the design records:

- The crate split shipped. The tree is a workspace of 16 crates plus the root crate (17 packages in the library survey); the root `id` crate is the public facade, and the CLI lives in `crates/id-cli/`. The pre-split paths (`src/cli.rs`, `src/store.rs`, `src/tags.rs` and the rest) do not exist. Survey disagreement DES-2 recorded that `pkgs/id/README.md`'s "Project Structure" still described that source tree; the README and `ARCHITECTURE.md` now name the current paths (`crates/id-cli/src/cli/`, `crates/id-store/src/store.rs`, `crates/id-core/src/tuple.rs` and the rest).
- The event store shipped with concrete formats: store format marker `id.events.store.v9`, wire domain `id.events.v1`, events as p2panda-core headers in canonical CBOR with BLAKE3 operation IDs, a maximum of 32 `after` hashes per event (`MAX_AFTER`), and ingest outcomes of accepted, duplicate, fork, waiting and refused.
- The tag store shipped on iroh-docs alpha/omega namespace pairs, with its registry at `.iroh-meta/registry.json`, `REGISTRY_VERSION = 1`.
- The `id://` addressing design has a running demonstration: the #9106 walking skeleton (`id walk`) reads and writes records by `id://` address today. Its design role is covered in [Design: sync, replication and FileOps](sync-fileops.md).
- Still design only, with no shipped counterpart verified in the surveys: BitTorrent-style chunk retrieval for graph objects, the CapTP bridge spike (#6014), and cross-store friendly-name fan-out beyond the local-store resolution. The friendly-name records' shipped status beyond the local store is unverified in the survey material.
