# Roadmap, issue map and the design-record index

This chapter is the programme record for the ID book (tracking issue #13332). It states the roadmap as the design archive records it, maps the epics and trackers the records name, and then gives the full design-record index: all 408 ID design records in the repo-root `doc/` archive, each mapped to exactly one design chapter of this book. The index is the coverage proof for the design-record dimension of the book. Counts in this chapter come from the archive survey for this book, verified in the tree at checkout `1ccb4a19457` on 2026-11-23: the archive holds 570 top-level entries and 2,917 paths in total; 408 entries are ID-programme records; 162 entries belong to other programmes (Atlas, Fleet, the PM stack, the home cluster) or are standing non-record material, and are excluded from the index.

## The roadmap in phases

The archive's own phase records are the roadmap's spine. Twenty records are homed in this chapter: the phase records, the remaining-work record, the platform-completion brief, the programme essay answers, the Phase 12 exit audits, the walking-skeleton record, and the investigation charter that commissioned this book.

- Phases 0–8 (2026-10-11 to 2026-10-13): `2026-10-11T19-04-32Z_phase_0_baseline` (baseline, toolchain, dependencies), `2026-10-11T22-25-52Z_phase_1_events_storage` (events, storage, crash safety, the identity store), `2026-10-12T04-03-47Z_phase_2_membership` (membership, auth frontier, delegation), `2026-10-12T08-58-59Z_phase_3_sync_relays` (sync, blobs, relays), `2026-10-12T14-33-36Z_phase_4_groups_encryption` (groups, encryption, history), `2026-10-12T14-33-36Z_phase_5_tags_documents` (tags, documents, search), `2026-10-12T19-11-31Z_phase_6_invites_pull_cli` (invites, pull, the CLI), `2026-10-12T21-22-53Z_phase_7_connectivity` (connectivity for later servers), `2026-10-13T23-44-27Z_phase_8_open_design_items` (open design items, retention, multi-device, views).
- `2026-10-11T11-08-42Z_design_remaining_work` (2026-10-11): the design remainder — decisions, open questions and status — a bundle record of 745 files including the task breakdowns and research harnesses.
- `2026-10-14T_platform_completion` (2026-10-14): the platform completion acceptance brief.
- `2026-10-22T15-25-00Z_handoff_research_workflow_pipeline` (2026-10-22): the research-to-delivery pipeline and platform backlog handoff.
- Phase 12 (#534): the platform-flexibility phase, items F12.1–F12.17. Its design record is `2026-11-01_design_id534.md`; its research package is `2026-11-03T00-05-09Z_research_phase-12-package-534` (18 files). Children named in the records include #807 (manifest), #835 (roles v2 and moderator wiring), #1061 (provider registry), #1062 (first providers), #1063 (audit log), #1064 (ABI), #1065 (guest SDKs), #1066 (program upgrade), #1067 (fair scheduling and quotas), #1068 (portals) and #1069 (module trust). Exit evidence: `2026-11-22T07-05-00Z_phase_id_phase12_exit_audit` and its re-run `2026-11-22T14-45-14Z_phase_id_phase12_exit_audit_rerun` (both 2026-11-22), plus `2026-11-23T02-02-09Z_reference_authority_boundary_audit` (the authority-boundary audit, Phase 12 exit criterion 9).
- Phase 13 (F13.x): the shell and client phase. Records named in the survey: #785 (UDP-denied SSH fallback), #786 (presentation tiers), #787 (kitty keyboard), #788 (native reference shell), #789 and #5532 (client capability providers), #790 (topology — closed and implemented per the essay-answers record). Phase 14 is the next-horizons phase that contains the #6278 ecosystem, #6300–#6303 agents, #6304 scale and more-shells topics, under core-topic parent #6280 and plan of record #6312 where the records state them.
- `2026-11-04_id-program-essay-answers.md` (2026-11-04): answers to the user's 2026-11-04 ID programme questions — the programme's own statement of where things stand at that date.
- `2026-11-22T16-13-20Z_design_id-walking-skeleton` (2026-11-22): the walking skeleton for #9106 — the path through the 126 rows.
- `2026-11-23T12-06-42Z_design_id_program_investigation_prompt` (2026-11-23): the charter record of #13240, the full-investigation, design-completion and build-out programme that commissioned this book. It defines ID, in one sentence, as a local-first, federated, capability-based application platform — the definition this book uses.

## The issue map: epics and trackers

Every epic and tracker below is named inside the design records, per the archive survey. Issue states are given only where a record states them, with the record's date.

### The core epic: #2056

#2056 is the core ID programme epic: core build, first apps, comms, currency, hosting, notes, chat, boards. Most design records state "Programme: epic #2056" in their headers. At the #13240 charter's writing (2026-11-23), 27 of its 29 sub-issues were closed; the two open sub-issues were #2036 (messaging and relay protocol v1) and #2039 (match attestation and audit). Sub-epics that function as epics in the records: #2036 (messaging/relay), #2039 (match attestation), #2043 (currencies), #2046 (serverless edition), #2048 (the Zettelkasten layer), #2059 (native spill, parent of #2819).

### Federation and primitives: #9106

#9106 covers extensible primitives, federation and namespaces: Part A (core), Part B (federation and trust), Part C (Atlas-facing, which is #9109: storage economics, ID search, recommendations). The shipped `id walk` skeleton is #9106's demonstration path. The related store-conformance thread is #9110 (the port contract; slices #10191 and #10213; adoption call #9082). The federation cluster runs #9129–#9148 and includes #9117 (the `id://` URI scheme), #9141 (peer trust classes) and #9144 (federated query fan-out).

### The programme trackers

| Tracker | Programme | Where it is treated in this book |
|---|---|---|
| #5678 | Knowledge layer (parent of #5842 and the #5843–#5848 / #6435 cluster) | [Design: the knowledge layer](knowledge-layer.md) |
| #5685 | DoS suite and performance benchmark programme (children #5891–#5897) | [Design: scale, performance and adversarial hardening](scale-hardening.md) |
| #5686 | Provider model (children include #5898, #5899, #5900, #5902, #5904, #5905) | [Design: capabilities and providers](capabilities-providers.md) |
| #5671 / #5672 | Transport fallback and storage decomposition (children include #5793, #5800, #5807) | [Design: sync, replication and FileOps](sync-fileops.md), [Design: hosted and serverless editions](hosted-editions.md) |
| #5955 | Worlds design programme (Roc libraries, dynamic user functions, presentation assets; children include #5973, #5985, #5986, #6006) | [Design: worlds and the guest runtime](worlds-runtime.md), [Design: the knowledge layer](knowledge-layer.md) |
| #6278 | Program ecosystem (children #6291, #6293, #6294, #6308) | [Design: first apps and the ecosystem](first-apps.md) |
| #6300–#6303 | Agents in worlds, the MCP bridge series (under core topic #6280; registry step #10662) | [Design: agents in worlds and the MCP bridge](agents-mcp.md) |
| #6304 | Scale programme (measurements #13083–#13088; v2 is #6305) | [Design: scale, performance and adversarial hardening](scale-hardening.md) |
| #9078 | Hosted editions programme (components E #9083, F #9084, I #9087 are the ID parts; components A and C are not ID) | [Design: hosted and serverless editions](hosted-editions.md) |
| #3399 → #3400–#3404 | The jurisdiction chain. The #3404 slice (fenced jurisdiction migration control plane) was the charter record's flag-held landing; its core landed on `main` on 2026-11-23 (PR #13347) and the issue stays open on the hosted wiring. | [Design: residency, jurisdiction and boundaries](residency-jurisdiction.md) |
| #3231 / #3232 | One receipt contract, and its Phase 3 audit | [Design: currency, tokens and metering](currency-metering.md), [Design: identity, directory, groups and delegation](identity-directory-groups.md) |
| #2898 | Reconstruction is admission | [Design: capabilities and providers](capabilities-providers.md) |
| #5545 | Writer-set view projection and discovery query | [Design: sync, replication and FileOps](sync-fileops.md) |
| #13240 | The full-investigation and build-out programme that commissioned this book | This chapter |
| #13332 | This book | This chapter |

Early threads named in the records: #1793 (tags provenance and listing origin; children include #4973), #2259 (friendly-name references), #2343 and #2344 (MQTT device bridge, speech capabilities), #2380 and #2382 (keyset authorization log, guardian recovery).

Sibling programme trackers, named in the archive's own map so the roadmap can point sideways: #13241 (Atlas, under epic #8983), #13264 (mcpx, under #8993), #13272 (Fleet). Those programmes have their own books and are not treated here.

## How the index is built

The index below transforms the archive survey's inventory table. Rules:

- One row per record; every record is listed exactly once; 408 rows in total.
- The chapter column of the survey becomes the book chapter, with one stated change: the survey's combined theme "Knowledge layer & first apps" (57 records) is split by this book into [Design: the knowledge layer](knowledge-layer.md) (31 records: notes, documents, tags, links, import, export, query, recommendations, AI over notes, and the worlds-adjacent programming-model and library records) and [Design: first apps and the ecosystem](first-apps.md) (26 records: the first app, chat, boards, match play, games, and the #6278 ecosystem ladder). 31 + 26 = 57; no record changes any other survey assignment.
- Record names are the `doc/` folder or file names, copied exactly. Dates are the record's date in the survey; three records carry no date in the archive (`2026-10-14T_platform_completion`, `2026-11-04_id-program-essay-answers.md`, and the three undated knowledge-layer records) and show `—`.
- Subjects are each record's own first heading, as extracted in the survey.
- Three records are bundles counted once each: `2026-10-11T03-44-33Z_design_groups_events_delegation` (407 files), `2026-10-11T11-08-42Z_design_remaining_work` (745 files) and `2026-11-13T03-40-00Z_design_id-atlas-summit` (104 files). The remaining 405 records are single files or small folders.

Per-chapter counts: foundations 32; worlds and the guest runtime 20; capabilities and providers 44; identity, directory, groups and delegation 56; sync, replication and FileOps 39; residency, jurisdiction and boundaries 11; hosted and serverless editions 30; shells and client surfaces 43; agents in worlds and the MCP bridge 6; currency, tokens and metering 26; the knowledge layer 31; first apps and the ecosystem 26; scale, performance and adversarial hardening 24; this roadmap chapter 20. Total: 408.

## What exists today

Grounding for this chapter is the archive itself. The 408 records exist as files in `doc/` at the surveyed checkout; the phase records, exit audits and essay answers are records of work and decisions, and each record's own status line governs what it claims — a design record is not an implementation, and the index above does not promote any record's status. The book itself (#13332) is the live item: at the time of writing, its chapters are being written from the four surveys, and the disagreements the surveys numbered (CLI-1 to CLI-14, LIB-1 to LIB-15, WEB-1 to WEB-15, DES-1 to DES-12; 56 in total) are filed for grouping after the book merges, per the book skeleton. Open programme items named by the records and still open at their records' dates include #2036 and #2039 (under #2056), the #3404 landing (flag-held), the #6304 chain #13087 and #13088, and the v4 agent steps #10663, #10664 and #10665. Their live state on 2026-11-23 beyond what the records state is unverified in this chapter.

## The design-record index

### Design: foundations — objects, graph and the file core — 32 records

| # | Record (`doc/` entry) | Date | Subject (record's own first heading) |
|---|---|---|---|
| 57 | `2026-10-17T15-00-00Z_architecture_next_stage_platform` | 2026-10-17 | Next stage of the platform: foundations, then flexibility |
| 81 | `2026-10-29T12-50-36Z_design_id_app_packaging` | 2026-10-29 | Design: ID app packaging — app manifest, folder layout, TUI/web parity |
| 88 | `2026-10-30T13-25-00Z_design_friendly_name_references.md` | 2026-10-30 | Design: friendly-name reference resolution — local store (spec for the #2259 PoC) |
| 93 | `2026-10-30T19-28-00Z_investigation_bittorrent_chunk_retrieval_for_graph_objects` | 2026-10-30 | Investigation: BitTorrent-Style Chunk Retrieval for Graph Objects (LIST 14b) |
| 103 | `2026-11-01T15-42-00Z_architecture_id_crate_split` | 2026-11-01 | `pkgs/id` crate split: Phase 1 architecture |
| 107 | `2026-11-01T16-53-00Z_design_id_crate_split` | 2026-11-01 | pkgs/id crate split: a workspace of importable crates |
| 123 | `2026-11-01_design_id2146.md` | 2026-11-01 | Design: programs as websites in `id serve` |
| 124 | `2026-11-01_design_id2240.md` | 2026-11-01 | Design: session header + segment checkpoint event kind |
| 125 | `2026-11-01_design_id2259.md` | 2026-11-01 | Design: local friendly-name reference resolution (#2259) |
| 139 | `2026-11-01_design_id560.md` | 2026-11-01 | Design: F12.18 denial-of-service suite in CI |
| 146 | `2026-11-01_design_id805.md` | 2026-11-01 | Design: F12.11 delta and binary frames |
| 157 | `2026-11-02T11-47-29Z_design_web_listeners` | 2026-11-02 | Web listener bindings for `id serve` |
| 166 | `2026-11-04T05-11-11Z_feature_structured_logging` | 2026-11-04 | Structured operational logging for `id` — design record |
| 169 | `2026-11-04T10-46-17Z_design_listing_origin_and_upload_time` | 2026-11-04 | File listing provenance: origin node and upload time in List |
| 170 | `2026-11-04T15-33-00Z_design_friendly_name_fanout.md` | 2026-11-04 | Design: friendly-name reference resolution v2 — cross-store fan-out (first slice) |
| 171 | `2026-11-04T21-50-26Z_feature_dynamic_publications_runtime` | 2026-11-04 | Dynamic publication runtime apply and operator CLI |
| 187 | `2026-11-07T04-33-34Z_architecture_secure_public_web_routers` | 2026-11-07 | Secure public web router constructors |
| 201 | `2026-11-08T09-12-27Z_design_id-self-hosting-interop-stance` | 2026-11-08 | Self-hosting and interop stance |
| 211 | `2026-11-08T10-28-00Z_design_id6009_public_domain_normalization` | 2026-11-08 | Public-domain storage normalization — CC0-1.0 for content, 0BSD for code |
| 218 | `2026-11-08T13-20-00Z_research_id6014_ocapn_captp_watch` | 2026-11-08 | Watch record: OCapN / CapTP stability and the CapTP bridge spike (issue #6014) |
| 229 | `2026-11-08T23-17-06Z_design_id_setup_ceremony` | 2026-11-08 | Native ID creation & setup ceremony — decision record |
| 283 | `2026-11-09T07-20-00Z_design_atlas_id_publication` | 2026-11-09 | Atlas → ID-graph publication format: corpus objects + provenance/lineage relations as ID-graph objects |
| 284 | `2026-11-09_audit_3068-phase3-closing.md` | 2026-11-09 | Phase 3 closing audit — parsed-shape contract (#3068 / #3069) |
| 321 | `2026-11-11T03-19-54Z_refactor_markdown_module_split` | 2026-11-11 | Refactor record: split of `pkgs/id/src/web/markdown.rs` (#4704) |
| 337 | `2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces` | 2026-11-13 | Design (proto): ID/Atlas extensible primitives, federation, namespaces |
| 338 | `2026-11-13T03-40-00Z_design_id-atlas-summit` | 2026-11-13 | Summit decisions, defaults and owner questions |
| 341 | `2026-11-13T10-15-00Z_design_scaling_technique_comparisons` | 2026-11-13 | Scaling-technique comparisons — design decision record |
| 359 | `2026-11-16_decision_id5753_kdl_fork_section5.md` | 2026-11-16 | Decision record — KDL fork, #5753 §5 design pass (payload grammar, extension marking, serde shape) |
| 369 | `2026-11-17T04-55-00Z_design_id9117_id_uri_scheme.md` | 2026-11-17 | Design — the `id://` URI scheme and per-namespace handlers/encodings (#9117) |
| 374 | `2026-11-17T22-00-00Z_design_id9114_related_path_links.md` | 2026-11-17 | Design — related path links in `hash.metadata` (#9114, S3.21–S3.22) |
| 378 | `2026-11-17_design_id9127_append_list_user_attributes.md` | 2026-11-17 | Status: design record for issue #9127, written 2026-11-17. |
| 405 | `2026-11-23_design_id9128_social_app_reference_model.md` | 2026-11-23 | Status: design record for issue #9128, written 2026-11-23. |

### Design: worlds and the guest runtime — 20 records

| # | Record (`doc/` entry) | Date | Subject (record's own first heading) |
|---|---|---|---|
| 9 | `2026-10-07T00-20-00Z_architecture_roc_worlds` | 2026-10-07 | Roc-Defined Multiplayer Worlds |
| 13 | `2026-10-10T03-56-25Z_feature_world_isolation` | 2026-10-10 | World isolation modes |
| 15 | `2026-10-10T04-59-57Z_feature_cross_world_access` | 2026-10-10 | Cross-world access: writes, force, and file scope |
| 32 | `2026-10-14T19-39-30Z_architecture_portable_world_surfaces` | 2026-10-14 | Portable world surfaces and controlled execution profiles |
| 33 | `2026-10-14T20-00-05Z_feature_source_only_world_installs` | 2026-10-14 | Source-only world module installation |
| 36 | `2026-10-15T09-27-28Z_feature_roc_json` | 2026-10-15 | Pure-Roc JSON for portable world programs |
| 39 | `2026-10-16T09-00-00Z_feature_world_module_provenance` | 2026-10-16 | Observed world module provenance |
| 40 | `2026-10-16T09-30-00Z_feature_world_module_provenance` | 2026-10-16 | World module build provenance |
| 51 | `2026-10-16T18-30-00Z_feature_world_authoring_cli` | 2026-10-16 | Authoring CLI: `id world new` and `id world stats` |
| 56 | `2026-10-17T10-00-00Z_feature_scheduled_events` | 2026-10-17 | Scheduled events: a world can ask for one fire at a set time |
| 60 | `2026-10-18T00-00-00Z_reference_platform_state` | 2026-10-18 | Platform world: current state, findings, and open work |
| 62 | `2026-10-18T18-40-00Z_report_platform_world_state` | 2026-10-18 | Platform world: state report |
| 96 | `2026-10-30T19-36-00Z_investigation_world_vm_wasm_portability` | 2026-10-30 | World-VM Wasm portability audit — what of the worlds runtime runs inside a Worker |
| 106 | `2026-11-01T16-09-48Z_feature_world_json_transcript` | 2026-11-01 | `id world join --command-mode --json` transcript |
| 138 | `2026-11-01_design_id558.md` | 2026-11-01 | Design: `id world dev` authoring loop |
| 152 | `2026-11-02T00-53-51Z_feature_world_host_view` | 2026-11-02 | Authenticated, read-only world host view |
| 161 | `2026-11-03T01-31-11Z_design_recurring_cross_world_schedules` | 2026-11-03 | Recurring and cross-world scheduled events |
| 176 | `2026-11-05T15-08-16Z_research_id5986_agpl_gpl_hosting` | 2026-11-05 | AGPL/GPL hosting: inspectable worlds, uploads, and self-hosting |
| 199 | `2026-11-08T09-10-00Z_research_worlds_conversation_archaeology` | 2026-11-08 | Conversation archaeology: prior discussions with the user about Second Life and programmable worlds, tied into the worlds program |
| 368 | `2026-11-16_id_world_dev_authoring.md` | 2026-11-16 | Authoring Roc worlds with `id world dev` |

### Design: capabilities and providers — 44 records

| # | Record (`doc/` entry) | Date | Subject (record's own first heading) |
|---|---|---|---|
| 10 | `2026-10-09T09-59-07Z_design_attenuated_capabilities` | 2026-10-09 | Attenuated capabilities for world access |
| 43 | `2026-10-16T11-00-00Z_feature_world_capability_ceilings` | 2026-10-16 | World capability ceilings |
| 44 | `2026-10-16T12-00-00Z_feature_world_cli_controls` | 2026-10-16 | World Operator CLI Controls for Capability Ceilings and Resource Profiles |
| 48 | `2026-10-16T16-00-00Z_feature_roc_cap` | 2026-10-16 | RocCap: shared request bookkeeping for the capability protocol |
| 50 | `2026-10-16T18-00-00Z_feature_records_readback` | 2026-10-16 | Records readback: a `records.read` capability |
| 65 | `2026-10-25T05-37-57Z_design_client_capability_providers` | 2026-10-25 | Client-side capability providers under the ceiling model (F13.7) |
| 83 | `2026-10-29T14-05-00Z_investigation_capability_taxonomy` | 2026-10-29 | Capability taxonomy v0 — interface specialization across web, ESP32/MQTT and text/audio |
| 111 | `2026-11-01T20-45-00Z_feature_admin_audit` | 2026-11-01 | F12.6 slice 1: administrative and provider-use audit |
| 112 | `2026-11-01_design_id2028.md` | 2026-11-01 | Design: ID core v1 — app model and minimum capabilities |
| 131 | `2026-11-01_design_id2898.md` | 2026-11-01 | Design: reconstruction is admission in `pkgs/id` |
| 141 | `2026-11-01_design_id786.md` | 2026-11-01 | Design: F13.4 presentation capability tiers with in-band terminal detection |
| 144 | `2026-11-01_design_id789.md` | 2026-11-01 | Design: Phase 13 F13.7 client-side capability providers |
| 163 | `2026-11-03T10-06-40Z_architecture_provider_registry_slice1` | 2026-11-03 | F12.4 Provider Registry — Slice 1 |
| 168 | `2026-11-04T06-40-00Z_feature_loc-capability.md` | 2026-11-04 | `loc.*` — location capabilities with an accuracy ceiling and per-use consent |
| 179 | `2026-11-05T17-24-09Z_design_id_license_detection_pipeline` | 2026-11-05 | Design: upload-time license detection pipeline |
| 184 | `2026-11-06_decision_id_tagging_triples.md` | 2026-11-06 | Decision: ID tag claims as atomic-triple projections |
| 185 | `2026-11-06_design_id5985.md` | 2026-11-06 | Design: license combination checker — combination events, v1 incompatibility rules, override vs exception-request flows, audit records |
| 190 | `2026-11-08T01-10-24Z_design_id_capability_certification_exceptions` | 2026-11-08 | Design: capability-certification exception policy |
| 192 | `2026-11-08T06-25-00Z_design_tag_scope` | 2026-11-08 | Tag scope: local-only vs shared claims (#1793) |
| 198 | `2026-11-08T09-10-00Z_design_embedded_providers_decision.md` | 2026-11-08 | Decision record: embedded providers — engine & smartphone instantiation of the #5686 provider model |
| 204 | `2026-11-08T09-15-00Z_design_id_capability_model` | 2026-11-08 | Design: the capability model — requested vs discovered-used sets, registry schema, custom-compiled platforms |
| 208 | `2026-11-08T10-12-00Z_design_id_capability_diff_semantics` | 2026-11-08 | Capability-change diff semantics — user-meaningful classes and re-approval triggers |
| 212 | `2026-11-08T10-40-00Z_design_id_capability_observability` | 2026-11-08 | Design: capability observability — who can see which capability facts, where |
| 224 | `2026-11-08T17-12-39Z_design_id_package_import_policy` | 2026-11-08 | Design: pure-package imports and package classes across capability tiers |
| 240 | `2026-11-08_decision_id5904_offline_degraded_providers.md` | 2026-11-08 | Decision: offline & degraded operation for capability providers |
| 242 | `2026-11-08_decision_id5909_sql_pgq_capability.md` | 2026-11-08 | Decision: SQL/PGQ capability boundary for ID triples |
| 271 | `2026-11-08_design_id6015.md` | 2026-11-08 | Design: land/parcel capability domains — per-parcel policy granularity without tokenized land |
| 279 | `2026-11-09T05-00-00Z_feature_mqtt_device_bridge_id_device_v1` | 2026-11-09 | `id.device.v1` MQTT device bridge — schema and PoC (#2343) |
| 282 | `2026-11-09T06-25-00Z_research_id5989_prior_art_capability_systems` | 2026-11-09 | Research: prior art — capability systems, OCAP, Spritely Goblins/OCapN, CRDT shared worlds |
| 313 | `2026-11-10_design_id5977_license_taxonomy.md` | 2026-11-10 | Design: license taxonomy & classifier — the enumerated set, the warning ladder, and the tag format |
| 320 | `2026-11-11T02-55-00Z_design_used-capability-derivation` | 2026-11-11 | Used-capability derivation tool — spec |
| 324 | `2026-11-11T08-50-24Z_design_used_capability_derivation` | 2026-11-11 | Design: used-capability derivation tool — static call-graph scan producing the registry used-set (#5998) |
| 326 | `2026-11-11_decision_id5817_sensitivity_classification.md` | 2026-11-11 | Decision: sensitivity classification model for ID table queries (#5817) |
| 340 | `2026-11-13T09-26-00Z_design_pool_effort_system` | 2026-11-13 | Brief supplement — capabilities: per-capability routing, failover, and pools as models |
| 353 | `2026-11-16T09-03-20Z_decision_id5902_terminal_website_host_providers.md` | 2026-11-16 | Decision record: host providers — terminal & website instantiation of the #5686 provider model |
| 380 | `2026-11-17_runbook_id5532_f13-7_client_file_read_rollout.md` | 2026-11-17 | F13.7 rollout runbook — `client.file.read`, the first client-side capability provider |
| 382 | `2026-11-20T07-30-11Z_feature_speech-capabilities.md` | 2026-11-20 | `speech.*` — conversational audio capabilities |
| 389 | `2026-11-22_design_id1062-blob-provider.md` | 2026-11-22 | F12.5 `blob` provider — design for the second slice of #1062 |
| 390 | `2026-11-22_design_id1062-http-fetch-provider.md` | 2026-11-22 | F12.5 `http.fetch` provider — design for the third slice of #1062 |
| 391 | `2026-11-22_design_id1062-kv-provider.md` | 2026-11-22 | F12.5 `kv` provider — design for the first slice of #1062 |
| 392 | `2026-11-22_design_id1062-notify-provider.md` | 2026-11-22 | F12.5 `notify` provider — design for the fourth slice of #1062 |
| 393 | `2026-11-22_design_id1067-admission.md` | 2026-11-22 | F12.10 guest-burn admission — design for the second slice of #1067 |
| 400 | `2026-11-23T07-06-00Z_design_id_capability_class_assignments` | 2026-11-23 | Capability classes for the post-#5999 registrations — assignment design pass |
| 404 | `2026-11-23_design_id1067-account-quotas.md` | 2026-11-23 | F12.10 per-account quotas — design for the ownership slice of #1067 |

### Design: identity, directory, groups and delegation — 56 records

| # | Record (`doc/` entry) | Date | Subject (record's own first heading) |
|---|---|---|---|
| 11 | `2026-10-09T12-00-00Z_feature_directory_accounts_groups` | 2026-10-09 | Accounts, groups, contacts and delegation |
| 14 | `2026-10-10T04-58-36Z_design_signed_artifacts` | 2026-10-10 | Signed artifacts, home servers, and cross-world access |
| 17 | `2026-10-10T07-00-00Z_feature_petnames_invites` | 2026-10-10 | Petnames and invite links |
| 18 | `2026-10-11T03-44-33Z_design_groups_events_delegation` | 2026-10-11 | Events, groups, delegation, removal, and history |
| 34 | `2026-10-14T20-10-00Z_feature_world_keyboard_parity` | 2026-10-14 | Browser and SSH keyboard parity for world programs |
| 49 | `2026-10-16T17-00-00Z_feature_chat_history` | 2026-10-16 | Server-observed chat history and grantable platform modules |
| 54 | `2026-10-17T08-00-00Z_feature_world_roles_directory` | 2026-10-17 | Worlds as places, first slice: role-scoped invites and a machine-readable world list |
| 61 | `2026-10-18T04-00-00Z_fix_world_chat_visibility` | 2026-10-18 | Fix: world chat is visible only to participants who may chat or read history |
| 70 | `2026-10-29T10-45-00Z_investigation_ephemeral_live_data_retention` | 2026-10-29 | Ephemeral live data vs the chained event graph — retention and intermittent signing — investigation |
| 90 | `2026-10-30T19-11-04Z_investigation_trust_visibility_model` | 2026-10-30 | Investigation: the ID trust/visibility model — accept-all ingest, writer-set views |
| 94 | `2026-10-30T19-35-00Z_investigation_boards_model` | 2026-10-30 | ID boards — a modern phpBB on ID identity: board model sketch |
| 102 | `2026-10-31T18-03-36Z_investigation_native_tier_tradeoffs` | 2026-10-31 | Investigation: the native tier vs WASM — fuel, trust, and when each is right |
| 105 | `2026-11-01T16-06-00Z_research_in_session_command_toggle` | 2026-11-01 | D1-045 — In-session command toggle (prefix key + visible mode indicator) |
| 109 | `2026-11-01T17-52-12Z_feature_stable_note_revisions` | 2026-11-01 | Stable note identity and revision graph |
| 128 | `2026-11-01_design_id2804.md` | 2026-11-01 | Design: Issuer key rotation and compromise recovery |
| 132 | `2026-11-01_design_id3231.md` | 2026-11-01 | Design: one receipt contract for `pkgs/id` artifacts |
| 142 | `2026-11-01_design_id787.md` | 2026-11-01 | Design: kitty keyboard protocol on the raw input path |
| 148 | `2026-11-01_design_id835.md` | 2026-11-01 | Design: builder and moderator authority through world-session frames |
| 151 | `2026-11-02T00-31-45Z_design_module_trust` | 2026-11-02 | Design: module trust — admin trust list and install-time signature check |
| 153 | `2026-11-02T04-44-00Z_design_group_remove_modes` | 2026-11-02 | Explicit group removal modes |
| 158 | `2026-11-02T12-47-00Z_design_petname_unicode_confusable_skeleton` | 2026-11-02 | Petname anti-mimicry: Unicode UTS #39 confusable skeleton |
| 167 | `2026-11-04T06-07-11Z_feature_local_key_envelope` | 2026-11-04 | Local passphrase-wrapped identity key envelope |
| 172 | `2026-11-04_audit_id3232_receipt_validation.md` | 2026-11-04 | Receipt-validation audit for `pkgs/id` artifacts (#3232) |
| 174 | `2026-11-05T13-36-21Z_design_threat_list_feed_protocol` | 2026-11-05 | Design: shared threat-list feed protocol — versioned, signed, subscribable feeds with local override lists |
| 177 | `2026-11-05T15-10-00Z_architecture_legacy_artifact_receive_cutoff` | 2026-11-05 | Legacy artifact receive cutoff |
| 195 | `2026-11-08T08-19-40Z_design_id_knowledge_sharing_permissions` | 2026-11-08 | Design: Knowledge-layer sharing and permissions |
| 197 | `2026-11-08T09-06-08Z_design_id_permission_ux` | 2026-11-08 | Design: Permission UX for worlds — pre-connect disclosure, grant modes, and memory |
| 207 | `2026-11-08T10-03-00Z_design_id_recovery_codes` | 2026-11-08 | ID recovery codes (custody variant c) — decision record |
| 217 | `2026-11-08T12-50-00Z_design_id6004_lambdamoo_governance_arc` | 2026-11-08 | Design note: reserved admin authority — what LambdaMOO's governance arc says id must keep off the ballot |
| 220 | `2026-11-08T13-45-00Z_design_id_world_governance` | 2026-11-08 | Design: world governance — moderation-as-library, per-world parameterization, pre-approval |
| 223 | `2026-11-08T15-20-00Z_design_sign-request-nonce-domain.md` | 2026-11-08 | sign_request nonce domain — fallible signing contract (2026-11-08) |
| 226 | `2026-11-08T19-52-00Z_feature_groups_page` | 2026-11-08 | Local groups and grants page at /groups — design record |
| 263 | `2026-11-08_design_id5861.md` | 2026-11-08 | Design: board moderation hooks |
| 267 | `2026-11-08_design_id5871.md` | 2026-11-08 | Design: talk-page semantics — lifecycle, permissions, and binding authority |
| 274 | `2026-11-09T00-16-56Z_decision_id5900_provider_capability_grants.md` | 2026-11-09 | Decision: capability grants for providers — grant shape, sensitivity classes, approval flows |
| 280 | `2026-11-09T05-06-00Z_decision_id5801_encrypted_shares_key_distribution.md` | 2026-11-09 | Decision: encrypted shares & key distribution — share shape, key holders, rotation, recipient proofs |
| 287 | `2026-11-09_decision_id5905_provider_security_model.md` | 2026-11-09 | Decision: provider security model — identity, admission, impersonation, rogue providers, bridge compromise, capability confusion, ceiling-composition |
| 294 | `2026-11-09_design_id6005_moderation_pack_format.md` | 2026-11-09 | Design: moderation pack format — data-only packs with a static policy core, feed subscriptions, and per-world parameters |
| 302 | `2026-11-10T21-05-00Z_design_id6007_moderation_presets` | 2026-11-10 | Moderation preset tiers: named presets that compile to concrete rules |
| 311 | `2026-11-10_design_id5881_stepup_escrow.md` | 2026-11-10 | Design: 2FA step-up gates & org-escrowed credential proxying (id) |
| 312 | `2026-11-10_design_id5887.md` | 2026-11-10 | Design: user-held keys — key custody variant (b) for `id` |
| 319 | `2026-11-11T00-18-00Z_decision_id5898_provider_declaration_model.md` | 2026-11-11 | Decision: provider declaration model — manifest contents, signing, versioning, dynamic vs static capabilities |
| 325 | `2026-11-11T19-15-00Z_design_block_lookup` | 2026-11-11 | Design: block-list lookups that can say "the lookup failed" (#8750) |
| 327 | `2026-11-11_design_id5984_licensing_policy.md` | 2026-11-11 | Design: licensing policy — server ToS uploader grant, account-creation defaults, admin license policies |
| 330 | `2026-11-12T19-10-07Z_design_keyset_authorization_log` | 2026-11-12 | ADR: keyset / authorization log for `id` identity (#2380) |
| 332 | `2026-11-12T20-58-00Z_research_id_permission_precedents` | 2026-11-12 | Research: Permission-prompt precedents — how platforms present, remember, and revoke capability grants |
| 333 | `2026-11-12T22-55-00Z_design_seed_phrase_recovery` | 2026-11-12 | Seed-phrase export/restore for the identity key |
| 339 | `2026-11-13T09-00-00Z_design_blob_gate_authorization_wiring.md` | 2026-11-13 | Design: blob gate authorization wiring in `id serve` (#173) |
| 342 | `2026-11-13_design_id5886.md` | 2026-11-13 | Design: service-held keys — key custody variant (a) for `id` |
| 348 | `2026-11-14T18-50-00Z_design_id_invite_delegation` | 2026-11-14 | Design: generic invite delegation — encoding, depth, leakage, and group live-set semantics |
| 357 | `2026-11-16T11-00-00Z_design_guardian_recovery` | 2026-11-16 | Design: guardian social recovery — record formats, successor-key authority, threshold signing (#2382) |
| 361 | `2026-11-16_decision_id5819_owner_override_mechanics.md` | 2026-11-16 | Decision: owner override mechanics & the encryption boundary (#5819) |
| 370 | `2026-11-17T05-15-00Z_design_id9141_peer_trust_classes.md` | 2026-11-17 | Design (#9141): peer trust classes + signed-data acceptance rule |
| 379 | `2026-11-17_design_id_sign_time_refusal.md` | 2026-11-17 | Sign only what receipt would accept — the sign-time refusal contract for `pkgs/id` |
| 384 | `2026-11-21T04-30-00Z_design_id_delegation_cross_signing_ceremony` | 2026-11-21 | Design: delegation, transfer, and cross-signed identity ceremony |
| 399 | `2026-11-23T05-02-30Z_design_id_capability_change_notices` | 2026-11-23 | Design: capability-change notices on reconnect — presentation, choices, re-baselining, invite pinning |

### Design: sync, replication and FileOps — 39 records

| # | Record (`doc/` entry) | Date | Subject (record's own first heading) |
|---|---|---|---|
| 3 | `2026-03-22T00-00-00Z_feature_peer-discovery` | 2026-03-22 | Peer Discovery via iroh-gossip + distributed-topic-tracker |
| 12 | `2026-10-10T02-39-03Z_design_directory_sync` | 2026-10-10 | Directory sync: sessions, friend envelopes, and replication |
| 16 | `2026-10-10T06-00-00Z_research_identity_capabilities` | 2026-10-10 | Research: identity, capabilities, petnames, transport |
| 19 | `2026-10-11T04-48-13Z_feature_takeover_idle_connections` | 2026-10-11 | Takeover passes idle keep-alive connections to the successor |
| 21 | `2026-10-11T11-08-42Z_feature_n0_relay_default` | 2026-10-11 | n0 relays by default: one endpoint policy and dial timeouts |
| 73 | `2026-10-29T11-05-00Z_investigation_multi_hop_relay_and_p2p_media` | 2026-10-29 | Multi-hop relay and P2P media paths — investigation |
| 84 | `2026-10-29T16-21-00Z_design_id_sync_alpn_split` | 2026-10-29 | One `/id-sync/1` handshake, and why serve's range flights moved to |
| 85 | `2026-10-29T16-57-03Z_design_id_sync_range_store_seam` | 2026-10-29 | Wiring `SyncMethod::Range` under the `/id-sync/1` handshake: the per-call lock store seam |
| 87 | `2026-10-30T13-15-00Z_design_id_writer_set_view_projection` | 2026-10-30 | Design: writer-set objects, view projection, and the discovery query for `pkgs/id` |
| 98 | `2026-10-30T19-45-00Z_investigation_playable_posts` | 2026-10-30 | Embedded playable posts — HTML5 mini-games, async challenges, forkable succession saves |
| 108 | `2026-11-01T17-30-12Z_design_host_event_export` | 2026-11-01 | Host-event journal export for world fixtures |
| 115 | `2026-11-01_design_id2036.md` | 2026-11-01 | Design: ID messaging and relay protocol v1 |
| 129 | `2026-11-01_design_id2818.md` | 2026-11-01 | Design: Journaled region-seam handoff for ID worlds |
| 140 | `2026-11-01_design_id785.md` | 2026-11-01 | Design: UDP-denied SSH fallback for `id` |
| 147 | `2026-11-01_design_id806.md` | 2026-11-01 | Design: F12.13 journal replication (read replicas over a signed stream) |
| 155 | `2026-11-02T07-00-00Z_design_save_reservations` | 2026-11-02 | Save reservations: in-flight exclusion for same-file saves |
| 156 | `2026-11-02T08-13-35Z_design_peer-announcement-authentication` | 2026-11-02 | Authenticated peer announcements and bounded discovery |
| 162 | `2026-11-03T02-34-54Z_security_world_replication_authorization` | 2026-11-03 | World replication authorization |
| 164 | `2026-11-03T14-55-00Z_design_id4630_oversized_replication_entries.md` | 2026-11-03 | Design: replicating a journal whose `Checkpoint` entry exceeds the payload bound |
| 175 | `2026-11-05T13-38-00Z_design_id5791_transport_negotiation.md` | 2026-11-05 | Design: transport negotiation & fallback ordering for ID |
| 191 | `2026-11-08T05-48-47Z_design_input_event_model` | 2026-11-08 | Input event model for relayed apps (touch, mouse, hover) |
| 200 | `2026-11-08T09-10-14Z_design_id5899_provider_discovery_negotiation` | 2026-11-08 | Provider discovery & negotiation protocols — decision record |
| 214 | `2026-11-08T11-08-53Z_design_id5813_relay_proxy_protocol` | 2026-11-08 | Design: ID relay and proxy protocol |
| 215 | `2026-11-08T11-10-00Z_design_id_writer_sets_v2` | 2026-11-08 | Design: writer sets v2 — view-as-object, set algebra, federation, discovery v2 |
| 264 | `2026-11-08_design_id5862.md` | 2026-11-08 | Design decision record: board subscription, discovery & feed integration |
| 285 | `2026-11-09_decision_id5810_carrier_framing.md` | 2026-11-09 | Decision: framing and fragmentation across ID carrier classes |
| 288 | `2026-11-09_decision_id5908_payload_boundary.md` | 2026-11-09 | Decision: the payload boundary — decompose-vs-attach decision procedure |
| 292 | `2026-11-09_design_id5838.md` | 2026-11-09 | Design: multi-device sync cluster — knowledge layer (local-first sync, conflicts, offline, E2EE, attachments) |
| 298 | `2026-11-10T19-55-00Z_design_id5796_connect_proxy_traversal.md` | 2026-11-10 | Design: HTTP CONNECT proxy traversal for ID |
| 305 | `2026-11-10T21-50-00Z_research_id806_d1-031-journal-replication` | 2026-11-10 | D1-031 — F12.13 journal replication to signed read replicas: research package |
| 307 | `2026-11-10_decision_id5809_versioning_negotiation.md` | 2026-11-10 | Decision: versioning, negotiation & forward-compatibility policy for the ID wire format |
| 317 | `2026-11-10_rendezvous_discovery_udp_denied_design.md` | 2026-11-10 | Decision record: rendezvous & discovery for UDP-denied clients |
| 336 | `2026-11-12_design_id9004.md` | 2026-11-12 | Design: Joplin continuous sync — target/plugin bridge against the #5848 baseline |
| 345 | `2026-11-14T13-20-00Z_design_id5795_relay_trust` | 2026-11-14 | Relay trust model & broker authorization — decision record (#5795) |
| 355 | `2026-11-16T10-39-25Z_design_id5800_partial_replication_protocol` | 2026-11-16 | Partial replication protocol — subset snapshot, scoped updates, existence privacy, dangling references |
| 356 | `2026-11-16T10-40-00Z_design_id5756_multiplexed_streams_seam.md` | 2026-11-16 | Seam record — multiplexed persistent streams over one tunnel (#5756) |
| 360 | `2026-11-16_decision_id5808_postcard_wire_schema_v1.md` | 2026-11-16 | Decision: postcard wire schema v1 — envelope, discriminators, and golden vectors |
| 366 | `2026-11-16_design_id6287_discovery_registry_federation_v1.md` | 2026-11-16 | Discovery, registry & federation — v1 design (#6287) |
| 394 | `2026-11-23T00-24-00Z_design_journal_ack_modes` | 2026-11-23 | Selectable journal acknowledgement modes for world input |

### Design: residency, jurisdiction and boundaries — 11 records

| # | Record (`doc/` entry) | Date | Subject (record's own first heading) |
|---|---|---|---|
| 76 | `2026-10-29T11-43-02Z_investigation_do_jurisdiction_pinning` | 2026-10-29 | Investigation: Durable Object jurisdiction / data-location pinning for per-user stores |
| 126 | `2026-11-01_design_id2283.md` | 2026-11-01 | Design: jurisdiction-aware per-user hosted stores |
| 133 | `2026-11-01_design_id3397.md` | 2026-11-01 | Design: jurisdiction-partitioned hosted worlds and world blobs |
| 134 | `2026-11-01_design_id3398.md` | 2026-11-01 | Design: residency boundaries for native bridge, federation and relays |
| 149 | `2026-11-01_investigation_shared_world_residency.md` | 2026-11-01 | Investigation: shared-world residency and federation |
| 150 | `2026-11-01_jurisdiction_aware_user_store_design.md` | 2026-11-01 | Design: jurisdiction-aware hosted user stores |
| 203 | `2026-11-08T09-15-00Z_design_boundary_audit_appeals.md` | 2026-11-08 | Design decision record: boundary audit & appeals UX (id) |
| 244 | `2026-11-08_design_id5683.md` | 2026-11-08 | Design: residency & boundaries framework — declarations, evidence, and enforcement skeleton |
| 293 | `2026-11-09_design_id5876_residency_evidence_model.md` | 2026-11-09 | Design decision record: residency evidence model (id) |
| 297 | `2026-11-10T19-52-00Z_design_id_sanctions_boundary` | 2026-11-10 | Sanctions boundary data model — decision record for #5878 |
| 373 | `2026-11-17T21-55-00Z_design_id8851_boundary_inputs.md` | 2026-11-17 | Design: boundary enforcement inputs — declaration residence, evidence sources, verifier (#8851) |

### Design: hosted and serverless editions — 30 records

| # | Record (`doc/` entry) | Date | Subject (record's own first heading) |
|---|---|---|---|
| 31 | `2026-10-14T10-18-19Z_design_p7_2_cloudflare_workers.md` | 2026-10-14 | P7.2 design: Cloudflare Workers pkarr option |
| 68 | `2026-10-28T23-34-14Z_investigation_cloudflare_workers_fit` | 2026-10-28 | Investigation: Fitting the ID Platform onto Cloudflare Workers |
| 74 | `2026-10-29T11-27-00Z_investigation_bridge_semantics_hosted_native` | 2026-10-29 | Investigation: Bridge Semantics — Hosted (HTTPS/WS) Edition Syncing with Native Iroh Nodes |
| 75 | `2026-10-29T11-28-18Z_investigation_key_recovery_hosted` | 2026-10-29 | Key recovery for hosted ID users — Google SSO loss vs the ID keypair |
| 99 | `2026-10-30T22-45-00Z_design_hosted_provisioning_sso_binding.md` | 2026-10-30 | Design spec: hosted provisioning + SSO→ID binding |
| 118 | `2026-11-01_design_id2046.md` | 2026-11-01 | Design: ID serverless edition v1 on Cloudflare Workers |
| 178 | `2026-11-05T15-10-00Z_design_cloudflare_workers_rendezvous_relay.md` | 2026-11-05 | Design: Cloudflare Workers rendezvous/relay component (for #5794) |
| 205 | `2026-11-08T09-45-00Z_design_id5811_wss_https_adapter.md` | 2026-11-08 | Design: adapter — WebSocket/WSS + HTTPS fallback (browser / restrictive-network path) |
| 230 | `2026-11-08T23-22-53Z_design_id_adapter-conformance` | 2026-11-08 | Design: adapter conformance harness & interop matrix CI |
| 237 | `2026-11-08_decision_id5868_cost_limits_guardrails.md` | 2026-11-08 | Decision: cost & limits guardrails for serverless storage |
| 249 | `2026-11-08_design_id5803.md` | 2026-11-08 | Design: shared foreign data-store adapter contract for ID |
| 250 | `2026-11-08_design_id5804.md` | 2026-11-08 | Design: IndexedDB adapter for ID — origin-scoped addressing, live vs materialized reads, structured-clone type mapping, storage limits & eviction |
| 252 | `2026-11-08_design_id5824.md` | 2026-11-08 | Design record — id5824: Stage A — Redis hot store + regular-store system of record |
| 265 | `2026-11-08_design_id5866.md` | 2026-11-08 | Design: local-dev parity for the serverless edition |
| 266 | `2026-11-08_design_id5869.md` | 2026-11-08 | Design: backup, recovery & exit for serverless storage |
| 289 | `2026-11-09_design_id5783.md` | 2026-11-09 | Design: ESP32 hardware platform line — boards, provisioning, firmware lifecycle, OTA, power |
| 290 | `2026-11-09_design_id5815_security_binding.md` | 2026-11-09 | Design: security binding review across adapters & relays |
| 306 | `2026-11-10_decision_id5681_serverless_storage.md` | 2026-11-10 | Decision: serverless edition storage — parent synthesis for #5681 |
| 310 | `2026-11-10_design_id5865.md` | 2026-11-10 | Design: Turso/libSQL journal fit — how libSQL replication composes with the signed journal |
| 318 | `2026-11-11T00-16-00Z_design_id5807_redis_sqlite_export` | 2026-11-11 | Design: Redis→SQLite export + query-Redis-get-SQLite (from #5672) |
| 334 | `2026-11-12T23-04-00Z_design_hosted-editions-workers` | 2026-11-12 | Design intake — hosted editions: PM service, ID service, and fleet tool as workable Cloudflare Workers equivalents |
| 335 | `2026-11-12_decision_id5867_backend_migration.md` | 2026-11-12 | Decision: backend selection and migration for #5867 |
| 344 | `2026-11-14T13-10-00Z_design_id5812_adapter_record.md` | 2026-11-14 | Adapter decision record — WebRTC, TCP/TLS, local IPC (#5812) |
| 365 | `2026-11-16_design_id6283_hosted_worlds_v1.md` | 2026-11-16 | Hosted worlds & always-on hosting — v1 design (#6283) |
| 372 | `2026-11-17T16-55-00Z_design_hosted-tenancy-architecture` | 2026-11-17 | Design (component E): hosted tenancy architecture — per-user Turso databases + a central tenancy DB |
| 375 | `2026-11-17_decision_id5863_storage_contract.md` | 2026-11-17 | Decision: the serverless storage contract (journal, world state, indexes) |
| 376 | `2026-11-17_decision_id5864_d1_journal_fit.md` | 2026-11-17 | Design: D1 journal fit — ordering, sessions, and replication against the signed journal |
| 385 | `2026-11-21T21-40-00Z_design_self_hosted-tier-adapters` | 2026-11-21 | Self-hosted tier: storage, transport, and placement adapter design |
| 396 | `2026-11-23T03-04-00Z_design_content-addressed-media-data-plane` | 2026-11-23 | Content-addressed media/data plane: reference-based access and GC (component F) |
| 398 | `2026-11-23T05-00-00Z_design_bridge_v1_implementation` | 2026-11-23 | The hosted signed-object bridge v1 — implementation record (#3818) |

### Design: shells and client surfaces — 43 records

| # | Record (`doc/` entry) | Date | Subject (record's own first heading) |
|---|---|---|---|
| 7 | `2026-04-01T00-00-00Z_feature_image_upload` | 2026-04-01 | Image Upload: Paste, Drag-Drop, and Toolbar Insert |
| 37 | `2026-10-15T16-00-00Z_feature_world_viewport_and_paste` | 2026-10-15 | Per-session world viewport and paste policy |
| 38 | `2026-10-15T16-30-00Z_feature_terminal_buffer_capture` | 2026-10-15 | Terminal buffer capture interface |
| 45 | `2026-10-16T13-00-00Z_feature_world_apps_structured_migration` | 2026-10-16 | Structured World Apps Migration to RocJson and Screen |
| 46 | `2026-10-16T14-00-00Z_feature_screen_render_alignment` | 2026-10-16 | Portable Screen Rendering for Territory and a Host Allocator Alignment Fix |
| 52 | `2026-10-16T19-00-00Z_feature_guest_reflow` | 2026-10-16 | Guest-aware reflow: a `layout` entry point |
| 53 | `2026-10-16T19-30-00Z_feature_screen_widgets` | 2026-10-16 | Screen widgets and `RocText` |
| 64 | `2026-10-24T20-18-37Z_decision_ssh_input_modes` | 2026-10-24 | SSH input modes: raw by default, `cmd:` line mode, and the username grammar |
| 66 | `2026-10-25T05-54-40Z_reference_shell_contract` | 2026-10-25 | The id shell contract, version 1 |
| 101 | `2026-10-31T17-47-20Z_design_world_presentation_and_fuel` | 2026-10-31 | World presentation, rendering, and fuel: the current model |
| 110 | `2026-11-01T17-54-35Z_design_id_concurrent_world_sessions` | 2026-11-01 | F13.8 concurrent world sessions: client coordinator |
| 121 | `2026-11-01_design_id2054.md` | 2026-11-01 | Design: ID boards v1 — signed boards, portable themes, and playable posts |
| 135 | `2026-11-01_design_id3455.md` | 2026-11-01 | Design: a value cannot act on the terminal — one Cf contract for `pkgs/id` plain output |
| 143 | `2026-11-01_design_id788.md` | 2026-11-01 | Design: Native reference shell for `id` (F13.6) |
| 145 | `2026-11-01_design_id790.md` | 2026-11-01 | Design: F13.8 multi-server and multi-client topology |
| 159 | `2026-11-02T_reference_kitty_disambiguate_v1.md` | — | kitty-disambiguate-v1 profile reference |
| 193 | `2026-11-08T06-45-00Z_design_id_query_local_remote_v1` | 2026-11-08 | Querying local and remote ID servers — v1 design |
| 194 | `2026-11-08T08-10-00Z_design_id5782_graphical_client_tooling.md` | 2026-11-08 | Graphical client & tooling surface — v1 design |
| 209 | `2026-11-08T10-26-00Z_design_id5996_hot_swap_decision` | 2026-11-08 | Design: hot-swap alternatives — decision record for the v1 mechanism |
| 210 | `2026-11-08T10-26-00Z_design_id_per_user_function_mapping` | 2026-11-08 | Design: per-user function mapping — bot battles, strategy hot-switching, human+AI-partner composition |
| 227 | `2026-11-08T22-06-00Z_design_id_android_client` | 2026-11-08 | Design: a native Android client for ID |
| 232 | `2026-11-08_decision_id5849_semantic_markup_vocabulary_v1.md` | 2026-11-08 | Decision: semantic markup vocabulary v1 — element set and authoring syntax |
| 233 | `2026-11-08_decision_id5852_native_rendering_os_standard_controls.md` | 2026-11-08 | Decision: native rendering — assertion → OS-standard controls |
| 234 | `2026-11-08_decision_id5853_custom_gui_rendering_overrides.md` | 2026-11-08 | Decision: custom GUI rendering as first-class overrides |
| 235 | `2026-11-08_decision_id5854_per_target_composition.md` | 2026-11-08 | Decision: per-target composition — themes, overrides, assets, conditional blocks |
| 236 | `2026-11-08_decision_id5855_rendering_pipeline_conformance.md` | 2026-11-08 | Decision: rendering pipeline & conformance suite — compile/runtime split, per-target expected-output tests, author tooling/linting |
| 243 | `2026-11-08_decision_id5910_kdl_presentation_surfaces.md` | 2026-11-08 | Decision: KDL presentation surfaces — export, hand upload, agent statements |
| 246 | `2026-11-08_design_id5781.md` | 2026-11-08 | Design: Bevy incorporation — ID worlds as Bevy applications |
| 247 | `2026-11-08_design_id5785.md` | 2026-11-08 | Design: output safety beyond plain text — markup contexts, safe-plain as a user choice, ingest without evaluation, and the ANSI/OSC disposition |
| 251 | `2026-11-08_design_id5820.md` | 2026-11-08 | Design: client surfaces parity — scripting/API, agent, and CLI front doors over one query model |
| 268 | `2026-11-08_design_id5873.md` | 2026-11-08 | Design: terminal participation in object discussions |
| 270 | `2026-11-08_design_id6012_visual_feature_cost_ranking.md` | 2026-11-08 | Design: visual-feature cost ranking |
| 275 | `2026-11-09T01-10-00Z_design_id_search_ranking` | 2026-11-09 | Personalized search ranking for notes — decision record |
| 286 | `2026-11-09_decision_id5851_assertion_tailwind_mapping.md` | 2026-11-09 | Decision: web rendering — assertion → Tailwind mapping |
| 301 | `2026-11-10T20-40-00Z_design_more_shells_v1` | 2026-11-10 | More shells v1 — first-iteration design (mobile, native, hardware) |
| 308 | `2026-11-10_decision_id5818_remote_filtered_query.md` | 2026-11-10 | Decision: server-side filtered remote query execution |
| 309 | `2026-11-10_design_id5858.md` | 2026-11-10 | Design decision record: portable theme format & precedence |
| 347 | `2026-11-14T13-50-00Z_design_id_roc_library_imports` | 2026-11-14 | Design: Roc library imports for worlds — platform-requirement representation, subset checking, provenance, and import errors |
| 350 | `2026-11-14_decision_id5907_triple_decomposition.md` | 2026-11-14 | Decision: triple decomposition rules for KDL statements |
| 358 | `2026-11-16T13-00-00Z_design_id5964_dynamic_user_functions` | 2026-11-16 | Design: dynamic user functions — interface contract, compile pipeline, and v1 placement |
| 371 | `2026-11-17T15-40-00Z_design_id_federated_query_fanout` | 2026-11-17 | Federated query fan-out — design record (#9144) |
| 377 | `2026-11-17_design_id6010_semantic_cell_tag_model.md` | 2026-11-17 | Design: semantic cell/tag presentation model — one tagged state, many renderers |
| 397 | `2026-11-23T03-17-14Z_design_more_shells_v2` | 2026-11-23 | More shells v2 — outline: native & TUI completion |

### Design: agents in worlds and the MCP bridge — 6 records

| # | Record (`doc/` entry) | Date | Subject (record's own first heading) |
|---|---|---|---|
| 295 | `2026-11-09_design_id6300_agents_in_worlds_v1.md` | 2026-11-09 | Agents in worlds (MCP bridge) — v1 design + spike plan (#6300) |
| 315 | `2026-11-10_design_id6301_agents_as_citizens_v2_outline.md` | 2026-11-10 | Agents in worlds (MCP bridge) — v2 outline: agents as citizens (#6301) |
| 352 | `2026-11-14_design_id6302_shared_agent_worlds_v3_outline.md` | 2026-11-14 | Agents in worlds (MCP bridge) — v3 outline: shared agent worlds (#6302) |
| 367 | `2026-11-16_design_id6303_hosted_agent_services_v4_outline.md` | 2026-11-16 | Agents in worlds (MCP bridge) — v4 outline: hosted agent services (#6303) |
| 383 | `2026-11-20T13-35-00Z_design_id10662_hosted_agent_service_listing` | 2026-11-20 | Hosted agent service listings: design record for v4 step 1 |
| 403 | `2026-11-23T12-46-33Z_design_id10662_listing_record` | 2026-11-23 | Listing record + registry protocol — the v4 registry's record family, signing, versioning, suspension/delisting, and discovery query |

### Design: currency, tokens and metering — 26 records

| # | Record (`doc/` entry) | Date | Subject (record's own first heading) |
|---|---|---|---|
| 2 | `2026-03-21T05-47-48Z_feature_content_modes` | 2026-03-21 | Content Modes, Format Conversion & Media Viewing |
| 69 | `2026-10-28T23-34-15Z_investigation_tigerbeetle_currency_backend` | 2026-10-28 | TigerBeetle as the accounting backend for dynamic ID currencies — investigation |
| 77 | `2026-10-29T11-57-00Z_investigation_tigerbeetle_cluster_topology` | 2026-10-29 | TigerBeetle cluster topology decision — 3 vs 6 replicas, site placement, commit latency |
| 78 | `2026-10-29T12-10-00Z_design_refundable_grants_pending_transfers` | 2026-10-29 | Refundable grants via pending transfers — premium tokens that survive a refund |
| 79 | `2026-10-29T12-20-00Z_design_currency_centralised_exception` | 2026-10-29 | Design: currency as the centralised exception — the hosted edition's boundary to the currency service |
| 80 | `2026-10-29T12-42-21Z_design_currency_control_plane` | 2026-10-29 | Currency control plane — design: registry, account factory, gateway |
| 86 | `2026-10-30T11-55-00Z_design_grant_lifecycle.md` | 2026-10-30 | Grant lifecycle implementation spec — pending holds, fingerprint registry, clawback/shortfall, revocation policy |
| 89 | `2026-10-30T13-25-00Z_design_graph_token_formats_snapshot_anchoring.md` | 2026-10-30 | Design: graph token object formats + snapshot anchoring |
| 91 | `2026-10-30T19-18-00Z_investigation_token_issuance_id_graph` | 2026-10-30 | Token issuance in the ID graph — investigation: per-ID issuance, issuer snapshots, authorized conversion rates |
| 117 | `2026-11-01_design_id2043.md` | 2026-11-01 | Design: ID currencies v1 |
| 122 | `2026-11-01_design_id2063.md` | 2026-11-01 | Design: ID conversion engine — linked transfer pairs and rate authorisation |
| 127 | `2026-11-01_design_id2290.md` | 2026-11-01 | Design: RateAuthorisation as a signed ID-graph object |
| 165 | `2026-11-03T20-36-00Z_design_tigerbeetle_ledger_binding` | 2026-11-03 | Design: TigerBeetle LedgerBackend binding (`tb_ledger`) |
| 180 | `2026-11-05_design_id5828.md` | 2026-11-05 | Design: conversion & RateAuthorisation across the #5676 currency stages |
| 186 | `2026-11-07T03-55-49Z_feature_graph_currency_contracts` | 2026-11-07 | Graph-native currency event contracts |
| 222 | `2026-11-08T14-05-49Z_design_id2043_extension.md` | 2026-11-08 | Design: ID tokens v1 — extension for the user's 2026-11-08 rulings |
| 241 | `2026-11-08_decision_id5906_kdl_json_contract.md` | 2026-11-08 | Decision: KDL ⇄ JSON canonical conversion and round-trip contract |
| 253 | `2026-11-08_design_id5829.md` | 2026-11-08 | Design: currency load & correctness verification programme |
| 273 | `2026-11-09T00-10-25Z_research_id5966_gas_timeouts_multithreading` | 2026-11-09 | Research: per-function gas metering, timeouts, and multithreading for user functions |
| 296 | `2026-11-10T03-10-00Z_design_id_two_level_metering` | 2026-11-10 | Design: two-level metering — burst bucket + hard per-tick ceiling, per function |
| 328 | `2026-11-11_design_id6016_metering_record_flow.md` | 2026-11-11 | Design: per-function metering record shape and flow — where metering data goes if servers charge by function class |
| 343 | `2026-11-14T06-22-00Z_poc_currency_gateway_boundary_results` | 2026-11-14 | PoC results — currency gateway boundary (#2298) |
| 351 | `2026-11-14_design_id5823.md` | 2026-11-14 | Design record — id5823: currency backend port + invariant test-suite contract |
| 362 | `2026-11-16_design_id5825.md` | 2026-11-16 | Design record — id5825: currency autoscaling — stateless front-end, hot-account mitigations, scaling triggers |
| 363 | `2026-11-16_design_id5826.md` | 2026-11-16 | Design record — id5826: Stage B — multi-Turso sharding by currency |
| 364 | `2026-11-16_design_id5827.md` | 2026-11-16 | Design record — id5827: Stage C — TigerBeetle adapter + B→C cutover |

### Design: the knowledge layer — 31 records

| # | Record (`doc/` entry) | Date | Subject (record's own first heading) |
|---|---|---|---|
| 5 | `2026-03-23T12-00-00Z_feature_metadata_tags` | 2026-03-23 | Metadata Tag System |
| 104 | `2026-11-01T16-05-00Z_design_documents_session_frames` | 2026-11-01 | D1-040 — Documents as a session frame pair (web/CLI parity) |
| 119 | `2026-11-01_design_id2048.md` | 2026-11-01 | Design: Zettelkasten layer — multiplayer docs/wiki on ID |
| 154 | `2026-11-02T04-52-00Z_feature_note_link_backlink_projection` | 2026-11-02 | Note links and the head-based backlink projection |
| 181 | `2026-11-05_design_id5965.md` | 2026-11-05 | Design: compile economics — compiled-world/DLL caching, compile-cost penalties, premium gating |
| 182 | `2026-11-05_design_id5997.md` | 2026-11-05 | Design: effect-language borrowings — which Koka/Unison properties id tooling emulates, approximates, or skips |
| 188 | `2026-11-08T00-45-00Z_design_id_recommendations` | 2026-11-08 | ID recommendations across all object types — v1 design |
| 196 | `2026-11-08T08-53-12Z_design_onenote_importer` | 2026-11-08 | Design: OneNote import into the ID knowledge layer |
| 213 | `2026-11-08T11-06-39Z_design_id_draft_approval_flow` | 2026-11-08 | Design: Draft approval queues, outcomes, and rejected-draft forks |
| 221 | `2026-11-08T13-55-51Z_design_agent_note_placement` | 2026-11-08 | Design: where agent notes live — scratch, memory, repo, Zettelkasten, or ID graph |
| 225 | `2026-11-08T17-23-00Z_design_id5963_programming_model` | 2026-11-08 | Design: explicit platform contexts for pure libraries |
| 238 | `2026-11-08_decision_id5874_thread_resolution.md` | 2026-11-08 | Decision: thread resolution and the discussion→edit loop (resolved-into links and change proposals) |
| 254 | `2026-11-08_design_id5830.md` | 2026-11-08 | Design: note→board sharing model — embedded live references, version semantics, reply anchoring, backlinks, cross-server case |
| 255 | `2026-11-08_design_id5832.md` | 2026-11-08 | Design: multiplayer edit sessions — UX, lifecycle, and participation (issue #5832) |
| 256 | `2026-11-08_design_id5837.md` | 2026-11-08 | Design: knowledge graph view cluster — local/global, edges, filters, scale, live behaviour |
| 257 | `2026-11-08_design_id5845.md` | 2026-11-08 | Obsidian importer — design record for the #2048 object model |
| 258 | `2026-11-08_design_id5846.md` | 2026-11-08 | Design: Evernote importer into the #2048 object model |
| 259 | `2026-11-08_design_id5847.md` | 2026-11-08 | Design: importer design — Logseq into the #2048 object model |
| 260 | `2026-11-08_design_id5848.md` | 2026-11-08 | Design: importer design — Joplin into the #2048 object model |
| 269 | `2026-11-08_design_id5969.md` | 2026-11-08 | Design: library typing & upload taxonomy — subtypes, entity attachment, DLL vs compile-in, libraries by URL |
| 272 | `2026-11-08_design_id6439_knowledge-interchange-and-export.md` | 2026-11-08 | Design: ID knowledge-layer interchange and full-fidelity export |
| 277 | `2026-11-09T03-57-04Z_design_id_edit_fork_graph` | 2026-11-09 | Edit/fork graph for notes — decision record |
| 291 | `2026-11-09_design_id5821.md` | 2026-11-09 | ID queries: result shaping & cursor pagination — decision record |
| 299 | `2026-11-10T20-00-00Z_design_agent_note_placement.md` | 2026-11-10 | Design: where agent notes live — placement rule for zettel vs repo vs ID graph |
| 300 | `2026-11-10T20-12-00Z_design_id_content_model` | 2026-11-10 | Content model cluster — decision record for #5839 |
| 304 | `2026-11-10T21-45-00Z_design_ai_over_notes` | 2026-11-10 | AI-over-notes: source-grounded Q&A, citations, and read-authority |
| 316 | `2026-11-10_design_id6436.md` | 2026-11-10 | Design: importer design — Notion into the #2048 object model |
| 329 | `2026-11-11_roc_library_evidence_5995.md` | 2026-11-11 | Roc shared-library outputs and host loading: evidence for #5995 |
| 406 | `knowledge-layer-attachments-media-5844.md` | — | Knowledge layer — attachments & media cluster decision record (#5844) |
| 407 | `knowledge-layer-extensibility-5843.md` | — | Knowledge layer — extensibility cluster decision record (#5843) |
| 408 | `knowledge-layer-feature-matrix-6435.md` | — | Knowledge-layer feature matrix — issue #6435 (program #5678) |

### Design: first apps and the ecosystem — 26 records

| # | Record (`doc/` entry) | Date | Subject (record's own first heading) |
|---|---|---|---|
| 1 | `2026-03-20T05-30-00Z_feature_collaborative_cursor_enhancements` | 2026-03-20 | Collaborative Cursor Enhancements |
| 4 | `2026-03-23T00:00:00Z_feature_file_list_ui` | 2026-03-23 | File List UI Improvements |
| 8 | `2026-04-01T00-00-00Z_feature_multi_instance` | 2026-04-01 | Multi-Instance Support for `id` |
| 47 | `2026-10-16T15-00-00Z_feature_roc_grid` | 2026-10-16 | RocGrid: a bounds-checked grid library for world programs |
| 55 | `2026-10-17T09-00-00Z_feature_world_fixtures` | 2026-10-17 | Session recording to replayable fixtures |
| 71 | `2026-10-29T10-54-33Z_investigation_match_audit_netcode_replay` | 2026-10-29 | Match audit — netcode log format, replay validation, and cheating signals — investigation |
| 82 | `2026-10-29T13-04-50Z_investigation_didit_tracker_data_model` | 2026-10-29 | Investigation: the did-it tracker data model |
| 92 | `2026-10-30T19-20-40Z_investigation_match_result_attestation` | 2026-10-30 | Match-result attestation — mutual concession, witness adjudication, offline play — investigation |
| 97 | `2026-10-30T19-40-00Z_investigation_irc_first_chat_on_id` | 2026-10-30 | IRC-first chat on ID — channels, graph-linked objects, presence |
| 113 | `2026-11-01_design_id2031.md` | 2026-11-01 | Design: the first ID app — sticky notes + did-it tracker |
| 116 | `2026-11-01_design_id2039.md` | 2026-11-01 | Design: match attestation + audit v1 |
| 120 | `2026-11-01_design_id2051.md` | 2026-11-01 | Design: ID chat v1 |
| 183 | `2026-11-06T00-50-21Z_design_dynamic_world_tick_cost` | 2026-11-06 | Dynamic-world per-tick cost model and bot-battle benchmark |
| 189 | `2026-11-08T00-57-08Z_design_shader_asset_format` | 2026-11-08 | ID shader asset format — v1 design |
| 202 | `2026-11-08T09-14-46Z_research_id5988_prior_art_bot_games_programmable_worlds` | 2026-11-08 | Research: prior art — scripted-bot battle games and user-programmable worlds |
| 219 | `2026-11-08T13-45-00Z_design_id5857_curated_board_submissions` | 2026-11-08 | Design decision record: curated-board submissions — intake queues, intake roles, notifications |
| 261 | `2026-11-08_design_id5856.md` | 2026-11-08 | Design decision record: post-to-board flow & post semantics |
| 262 | `2026-11-08_design_id5859.md` | 2026-11-08 | Decision record: live-object embeds in playable posts |
| 276 | `2026-11-09T01-27-00Z_design_chat_anchor_data_model` | 2026-11-09 | Chat anchor data model: typed links and durable part selectors |
| 278 | `2026-11-09T04-43-05Z_design_id5788_build_variants.md` | 2026-11-09 | Feature-leveled build variants — design record |
| 303 | `2026-11-10T21-24-00Z_design_id_program_ecosystem` | 2026-11-10 | Program ecosystem (share, fork, gallery) — first-pass research + design for #6278 |
| 322 | `2026-11-11T05-01-15Z_design_id_program_ecosystem_v1` | 2026-11-11 | Program ecosystem — v1 design: share/fork basics for #6291 |
| 323 | `2026-11-11T05-24-00Z_design_id_program_ecosystem_v2` | 2026-11-11 | Program ecosystem — v2 design: gallery & updates for #6293 |
| 346 | `2026-11-14T13-33-00Z_design_id_program_ecosystem_v3` | 2026-11-14 | Program ecosystem — v3 design: the public gallery for #6294 |
| 349 | `2026-11-14T19-05-00Z_design_id_build_defaults` | 2026-11-14 | Design: build-time defaults file for id network defaults |
| 381 | `2026-11-18_design_id6308_creator_owner_basics_v1.md` | 2026-11-18 | Creator tooling & insight — owner basics v1 design (#6308) |

### Design: scale, performance and adversarial hardening — 24 records

| # | Record (`doc/` entry) | Date | Subject (record's own first heading) |
|---|---|---|---|
| 6 | `2026-03-29T00-00-00Z_reference_testing_architecture` | 2026-03-29 | Testing Architecture |
| 41 | `2026-10-16T10-00-00Z_feature_world_resource_profile` | 2026-10-16 | Per-world resource profiles |
| 42 | `2026-10-16T10-30-00Z_feature_durable_world_resource_profiles` | 2026-10-16 | Durable per-world resource profiles |
| 58 | `2026-10-17T15-30-00Z_feature_world_perf_budgets` | 2026-10-17 | Per-call resource metrics and performance budgets |
| 59 | `2026-10-17T16-30-00Z_fix_territory_view_fuel` | 2026-10-17 | Fix: territory's view ran out of fuel with six or more seats |
| 67 | `2026-10-27T00-00-00Z_baseline_bytes_per_frame.md` | 2026-10-27 | F11.9 — Bytes-per-frame baseline table |
| 72 | `2026-10-29T10-55-00Z_investigation_comms_abuse_controls` | 2026-10-29 | Comms abuse controls for ID — blocks, rate limits and per-counterparty budgets |
| 95 | `2026-10-30T19-35-00Z_investigation_hot_world_sharding` | 2026-10-30 | Hot-world sharding — investigation: when one world outgrows one Durable Object |
| 100 | `2026-10-31T04-00-00Z_design_waiting_pool_total_cap` | 2026-10-31 | Waiting Pool Total Cap — Bounding Memory and Disk Waiting Rows Against Sybil Floods |
| 114 | `2026-11-01_design_id2033.md` | 2026-11-01 | Design: abuse-control primitives v1 for ID comms |
| 130 | `2026-11-01_design_id2819.md` | 2026-11-01 | Design: Native-spill write lease protocol for hot worlds |
| 137 | `2026-11-01_design_id557.md` | 2026-11-01 | Design: F12.12 per-world memory budget and many-world scale |
| 206 | `2026-11-08T09-55-28Z_design_standard_server_baseline` | 2026-11-08 | The standard server baseline — design decision record |
| 216 | `2026-11-08T11-27-00Z_design_shared_workload_generator` | 2026-11-08 | The shared load & workload generator — design decision record |
| 228 | `2026-11-08T23-05-00Z_design_id5797_latency_budget_measurement.md` | 2026-11-08 | Design: per-fallback-tier latency budget & measurement plan for ID |
| 231 | `2026-11-08T23-45-30Z_design_id5897_regression_gates` | 2026-11-08 | Decision: benchmark regression gates, waivers, and result retention |
| 239 | `2026-11-08_decision_id5892_dos_scenario_catalogue.md` | 2026-11-08 | Decision Record: DoS Scenario Catalogue per Interface |
| 245 | `2026-11-08_design_id5779.md` | 2026-11-08 | Design: deterministic testing for ID across time, replay, corpus, and soak |
| 248 | `2026-11-08_design_id5787.md` | 2026-11-08 | Design: Spill v2 — native spill pools, lease failover, and multi-holder fencing |
| 281 | `2026-11-09T05-15-00Z_design_id_server_invariants` | 2026-11-09 | Design: server-side invariants — budgets, observability, exploit resistance, failure containment |
| 314 | `2026-11-10_design_id6002_cooperative_timeout_contract.md` | 2026-11-10 | Contract: cooperative timeout — invocation budget, checkpoint convention, hard-kill outcomes |
| 331 | `2026-11-12T19-35-00Z_feature_world_lease_handoff_actor_boundary` | 2026-11-12 | World lease handoff at the actor boundary |
| 354 | `2026-11-16T09-52-00Z_design_ci_tiering_adversarial_benchmark_suites` | 2026-11-16 | CI tiering for the adversarial and benchmark suites — design decision record |
| 401 | `2026-11-23T07-53-00Z_design_scale_perf_stress_v1` | 2026-11-23 | Scale & performance hardening — v1: large-world stress program design |

### Roadmap, issue map and the design-record index — 20 records

| # | Record (`doc/` entry) | Date | Subject (record's own first heading) |
|---|---|---|---|
| 20 | `2026-10-11T11-08-42Z_design_remaining_work` | 2026-10-11 | What is left in the design: decisions, open questions, and status |
| 22 | `2026-10-11T19-04-32Z_phase_0_baseline` | 2026-10-11 | Phase 0: baseline, toolchain, dependencies, and small fixes |
| 23 | `2026-10-11T22-25-52Z_phase_1_events_storage` | 2026-10-11 | Phase 1: events, storage, crash safety, and the identity store |
| 24 | `2026-10-12T04-03-47Z_phase_2_membership` | 2026-10-12 | Phase 2: membership, auth frontier, and delegation |
| 25 | `2026-10-12T08-58-59Z_phase_3_sync_relays` | 2026-10-12 | Phase 3: sync, blobs, and relays |
| 26 | `2026-10-12T14-33-36Z_phase_4_groups_encryption` | 2026-10-12 | Phase 4: groups, encryption, and history |
| 27 | `2026-10-12T14-33-36Z_phase_5_tags_documents` | 2026-10-12 | Phase 5: tags, documents, and search |
| 28 | `2026-10-12T19-11-31Z_phase_6_invites_pull_cli` | 2026-10-12 | Phase 6: invites, pull, and the CLI |
| 29 | `2026-10-12T21-22-53Z_phase_7_connectivity` | 2026-10-12 | Phase 7: connectivity for later servers |
| 30 | `2026-10-13T23-44-27Z_phase_8_open_design_items` | 2026-10-13 | Phase 8: open design items, retention, multi-device, and views |
| 35 | `2026-10-14T_platform_completion` | — | Platform completion: acceptance brief |
| 63 | `2026-10-22T15-25-00Z_handoff_research_workflow_pipeline` | 2026-10-22 | Handoff: the research-to-delivery pipeline and platform backlog |
| 136 | `2026-11-01_design_id534.md` | 2026-11-01 | Design: Run Phase 12 deliverables through the research workflow |
| 160 | `2026-11-03T00-05-09Z_research_phase-12-package-534` | 2026-11-03 | Changes |
| 173 | `2026-11-04_id-program-essay-answers.md` | — | Answers to the user's 2026-11-04 ID program questions |
| 386 | `2026-11-22T07-05-00Z_phase_id_phase12_exit_audit` | 2026-11-22 | Phase 12 exit audit — 2026-11-22 |
| 387 | `2026-11-22T14-45-14Z_phase_id_phase12_exit_audit_rerun` | 2026-11-22 | Phase 12 exit audit — re-run and reconciliation, 2026-11-22 |
| 388 | `2026-11-22T16-13-20Z_design_id-walking-skeleton` | 2026-11-22 | Walking skeleton for #9106: the path through the 126 rows |
| 395 | `2026-11-23T02-02-09Z_reference_authority_boundary_audit` | 2026-11-23 | Authority-boundary audit — Phase 12 exit criterion 9 |
| 402 | `2026-11-23T12-06-42Z_design_id_program_investigation_prompt` | 2026-11-23 | ID Program — Full Investigation, Design Completion & Build-Out Prompt |

Index ends. Row count check: 32 + 20 + 44 + 56 + 39 + 11 + 30 + 43 + 6 + 26 + 31 + 26 + 24 + 20 = 408 records, each listed exactly once.
