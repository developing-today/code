# Architecture index

This is the entry point for the repository's architecture record — the
place #6380 asked for. It is a maintained control document (it changes
in place, through PRs, per
[`planning/DOCUMENTATION_ARCHITECTURE.md`](../DOCUMENTATION_ARCHITECTURE.md));
the records it links are dated records in [`doc/`](../../doc/README.md)
that say what was known when they were written and are not rewritten.

## Architecture records (dated, in `doc/`)

- [Testing Architecture](../../doc/2026-03-29T00-00-00Z_reference_testing_architecture/2026-03-29T00-00-00Z_reference_testing_architecture.md)
- [Roc-Defined Multiplayer Worlds](../../doc/2026-10-07T00-20-00Z_architecture_roc_worlds/2026-10-07T00-20-00Z_architecture_roc_worlds.md)
- [Portable world surfaces and controlled execution profiles](../../doc/2026-10-14T19-39-30Z_architecture_portable_world_surfaces/2026-10-14T19-39-30Z_architecture_portable_world_surfaces.md)
- [Next stage of the platform: foundations, then flexibility](../../doc/2026-10-17T15-00-00Z_architecture_next_stage_platform/2026-10-17T15-00-00Z_architecture_next_stage_platform.md)
- [`pkgs/id` crate split: Phase 1 architecture](../../doc/2026-11-01T15-42-00Z_architecture_id_crate_split/2026-11-01T15-42-00Z_architecture_id_crate_split.md)
- [F12.4 Provider Registry — Slice 1](../../doc/2026-11-03T10-06-40Z_architecture_provider_registry_slice1/2026-11-03T10-06-40Z_architecture_provider_registry_slice1.md)

## Design records (dated, in `doc/`)

Design records carry the trade-offs behind the architecture; the full
set lives in [`doc/`](../../doc/README.md) — run `ls doc/` for the
complete folder list, including feature, phase, fix and research
records.

- [Attenuated capabilities for world access](../../doc/2026-10-09T09-59-07Z_design_attenuated_capabilities/2026-10-09T09-59-07Z_design_attenuated_capabilities.md)
- [Directory sync: sessions, friend envelopes, and replication](../../doc/2026-10-10T02-39-03Z_design_directory_sync/2026-10-10T02-39-03Z_design_directory_sync.md)
- [Signed artifacts, home servers, and cross-world access](../../doc/2026-10-10T04-58-36Z_design_signed_artifacts/2026-10-10T04-58-36Z_design_signed_artifacts.md)
- [Events, groups, delegation, removal, and history](../../doc/2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T03-44-33Z_design_groups_events_delegation.md)
- [What is left in the design: decisions, open questions, and status](../../doc/2026-10-11T11-08-42Z_design_remaining_work/2026-10-11T11-08-42Z_design_remaining_work.md)
- [Phase 8: open design items, retention, multi-device, and views](../../doc/2026-10-13T23-44-27Z_phase_8_open_design_items/2026-10-13T23-44-27Z_phase_8_open_design_items.md)
- [2026-10-19T06-09-02Z_design_research_workflow](../../doc/2026-10-19T06-09-02Z_design_research_workflow/2026-10-19T06-09-02Z_design_research_workflow.md)
- [Project-management stack: layered tools, thin glue, one aggregator](../../doc/2026-10-24T11-05-38Z_design_project-management-stack/2026-10-24T11-05-38Z_design_project-management-stack.md)
- [Client-side capability providers under the ceiling model (F13.7)](../../doc/2026-10-25T05-37-57Z_design_client_capability_providers/2026-10-25T05-37-57Z_design_client_capability_providers.md)
- [Route pools: tier pools over per-model plan pools](../../doc/2026-10-25T21-11-31Z_design_route-pools/2026-10-25T21-11-31Z_design_route-pools.md)
- [Fleet content-creation pacing: measured rates, the 2026-10-28 block, and a shared throttle](../../doc/2026-10-29T10-08-00Z_design_github-content-creation-pacing/2026-10-29T10-08-00Z_design_github-content-creation-pacing.md)
- [Design: a Muse↔OpenClaw comms path that is not laptop SSH](../../doc/2026-10-29T12-09-49Z_design_openclaw_comms_path/2026-10-29T12-09-49Z_design_openclaw_comms_path.md)
- [Refundable grants via pending transfers — premium tokens that survive a refund](../../doc/2026-10-29T12-10-00Z_design_refundable_grants_pending_transfers/2026-10-29T12-10-00Z_design_refundable_grants_pending_transfers.md)
- [Design: currency as the centralised exception — the hosted edition's boundary to the currency service](../../doc/2026-10-29T12-20-00Z_design_currency_centralised_exception/2026-10-29T12-20-00Z_design_currency_centralised_exception.md)
- [Currency control plane — design: registry, account factory, gateway](../../doc/2026-10-29T12-42-21Z_design_currency_control_plane/2026-10-29T12-42-21Z_design_currency_control_plane.md)
- [Design: ID app packaging — app manifest, folder layout, TUI/web parity](../../doc/2026-10-29T12-50-36Z_design_id_app_packaging/2026-10-29T12-50-36Z_design_id_app_packaging.md)
- [One `/id-sync/1` handshake, and why serve's range flights moved to](../../doc/2026-10-29T16-21-00Z_design_id_sync_alpn_split/2026-10-29T16-21-00Z_design_id_sync_alpn_split.md)
- [Wiring `SyncMethod::Range` under the `/id-sync/1` handshake: the per-call lock store seam](../../doc/2026-10-29T16-57-03Z_design_id_sync_range_store_seam/2026-10-29T16-57-03Z_design_id_sync_range_store_seam.md)
- [Design: writer-set objects, view projection, and the discovery query for `pkgs/id`](../../doc/2026-10-30T13-15-00Z_design_id_writer_set_view_projection/2026-10-30T13-15-00Z_design_id_writer_set_view_projection.md)
- [Waiting Pool Total Cap — Bounding Memory and Disk Waiting Rows Against Sybil Floods](../../doc/2026-10-31T04-00-00Z_design_waiting_pool_total_cap/2026-10-31T04-00-00Z_design_waiting_pool_total_cap.md)
- [World presentation, rendering, and fuel: the current model](../../doc/2026-10-31T17-47-20Z_design_world_presentation_and_fuel/2026-10-31T17-47-20Z_design_world_presentation_and_fuel.md)
- [D1-040 — Documents as a session frame pair (web/CLI parity)](../../doc/2026-11-01T16-05-00Z_design_documents_session_frames/2026-11-01T16-05-00Z_design_documents_session_frames.md)
- [pkgs/id crate split: a workspace of importable crates](../../doc/2026-11-01T16-53-00Z_design_id_crate_split/2026-11-01T16-53-00Z_design_id_crate_split.md)
- [Host-event journal export for world fixtures](../../doc/2026-11-01T17-30-12Z_design_host_event_export/2026-11-01T17-30-12Z_design_host_event_export.md)
- [F13.8 concurrent world sessions: client coordinator](../../doc/2026-11-01T17-54-35Z_design_id_concurrent_world_sessions/2026-11-01T17-54-35Z_design_id_concurrent_world_sessions.md)
- [PM stack forecasting: velocity, quota walls, and stall risk](../../doc/2026-11-01T21-07-00Z_design_pm-stack_forecasting/2026-11-01T21-07-00Z_design_pm-stack_forecasting.md)
- [Design: module trust — admin trust list and install-time signature check](../../doc/2026-11-02T00-31-45Z_design_module_trust/2026-11-02T00-31-45Z_design_module_trust.md)
- [Explicit group removal modes](../../doc/2026-11-02T04-44-00Z_design_group_remove_modes/2026-11-02T04-44-00Z_design_group_remove_modes.md)
- [Save reservations: in-flight exclusion for same-file saves](../../doc/2026-11-02T07-00-00Z_design_save_reservations/2026-11-02T07-00-00Z_design_save_reservations.md)
- [Authenticated peer announcements and bounded discovery](../../doc/2026-11-02T08-13-35Z_design_peer-announcement-authentication/2026-11-02T08-13-35Z_design_peer-announcement-authentication.md)
- [Web listener bindings for `id serve`](../../doc/2026-11-02T11-47-29Z_design_web_listeners/2026-11-02T11-47-29Z_design_web_listeners.md)
- [Petname anti-mimicry: Unicode UTS #39 confusable skeleton](../../doc/2026-11-02T12-47-00Z_design_petname_unicode_confusable_skeleton/2026-11-02T12-47-00Z_design_petname_unicode_confusable_skeleton.md)
- [Design: task-placement framework v1 — decision rules and a shared-pool efficiency model](../../doc/2026-11-03T15-10-56Z_design_task-placement-framework-v1/2026-11-03T15-10-56Z_design_task-placement-framework-v1.md)
- [Design: TigerBeetle LedgerBackend binding (`tb_ledger`)](../../doc/2026-11-03T20-36-00Z_design_tigerbeetle_ledger_binding/2026-11-03T20-36-00Z_design_tigerbeetle_ledger_binding.md)
- [Atlas as a Rust binary and a multi-client search service](../../doc/2026-11-04T08-07-13Z_design_atlas_binary_service/2026-11-04T08-07-13Z_design_atlas_binary_service.md)
- [File listing provenance: origin node and upload time in List](../../doc/2026-11-04T10-46-17Z_design_listing_origin_and_upload_time/2026-11-04T10-46-17Z_design_listing_origin_and_upload_time.md)
- [One `fleet` multicall binary: search, queue, dispatch, proxy](../../doc/2026-11-04T11-45-48Z_design_fleet_multicall_binary/2026-11-04T11-45-48Z_design_fleet_multicall_binary.md)
- [Atlas / Turso data architecture — Scaler-era revision](../../doc/2026-11-04T21-27-34Z_design_atlas_scaler_revision/2026-11-04T21-27-34Z_design_atlas_scaler_revision.md)
- [Atlas — schema-driven filter surface from the classification manifest](../../doc/2026-11-05T13-03-49Z_design_atlas_manifest_filter_surface/2026-11-05T13-03-49Z_design_atlas_manifest_filter_surface.md)
- [Design: shared threat-list feed protocol — versioned, signed, subscribable feeds with local override lists](../../doc/2026-11-05T13-36-21Z_design_threat_list_feed_protocol/2026-11-05T13-36-21Z_design_threat_list_feed_protocol.md)
- [OneNote import into the ID knowledge layer (#6438)](../../doc/2026-11-08T08-53-12Z_design_onenote_importer/2026-11-08T08-53-12Z_design_onenote_importer.md)
- [ID shader asset format — v1 design](../../doc/2026-11-08T00-57-08Z_design_shader_asset_format/2026-11-08T00-57-08Z_design_shader_asset_format.md)
- [Knowledge-layer sharing and permissions](../../doc/2026-11-08T08-19-40Z_design_id_knowledge_sharing_permissions/2026-11-08T08-19-40Z_design_id_knowledge_sharing_permissions.md)
- [Permission UX for worlds: pre-connect disclosure, grant modes, and memory](../../doc/2026-11-08T09-06-08Z_design_id_permission_ux/2026-11-08T09-06-08Z_design_id_permission_ux.md)
- [Pure-package imports and package classes across capability tiers](../../doc/2026-11-08T17-12-39Z_design_id_package_import_policy/2026-11-08T17-12-39Z_design_id_package_import_policy.md)
- [Explicit platform contexts for pure libraries](../../doc/2026-11-08T17-23-00Z_design_id5963_programming_model/2026-11-08T17-23-00Z_design_id5963_programming_model.md)
- [Capability observability: who can see which capability facts, where](../../doc/2026-11-08T10-40-00Z_design_id_capability_observability/2026-11-08T10-40-00Z_design_id_capability_observability.md)
- [ID relay and proxy protocol (#5813)](../../doc/2026-11-08T11-08-53Z_design_id5813_relay_proxy_protocol/2026-11-08T11-08-53Z_design_id5813_relay_proxy_protocol.md)
- [ID relay/proxy transport-boundary clarification (#5813)](../../doc/2026-11-08T11-08-53Z_design_id5813_relay_proxy_protocol/2026-11-09T00-07-24Z_design_id5813_relay_proxy_protocol_transport_boundary_clarification.md)
- [ID relay/proxy sibling contract alignment (#5813)](../../doc/2026-11-08T11-08-53Z_design_id5813_relay_proxy_protocol/2026-11-10T06-54-33Z_design_id5813_relay_proxy_protocol_sibling_alignment.md)
- [Server-side invariants: budgets, observability, exploit resistance, containment](../../doc/2026-11-09T05-15-00Z_design_id_server_invariants/2026-11-09T05-15-00Z_design_id_server_invariants.md)
- [Two-level metering: burst bucket + hard per-tick ceiling, per function](../../doc/2026-11-10T03-10-00Z_design_id_two_level_metering/2026-11-10T03-10-00Z_design_id_two_level_metering.md)
- [Framing and fragmentation across ID carrier classes](../../doc/2026-11-09_decision_id5810_carrier_framing.md) (decision for #5810: length-prefixed stream frames, datagram whole-frame-or-checked-chunk assembly, store-and-forward opaque-object-or-checked-chunk assembly, and the invariants each adapter must hold)
- [Draft approval queues, outcomes, and rejected-draft forks](../../doc/2026-11-08T11-06-39Z_design_id_draft_approval_flow/2026-11-08T11-06-39Z_design_id_draft_approval_flow.md)

## Package architecture docs (living, with the code)

- [id — architecture](../../pkgs/id/ARCHITECTURE.md)
- Package docs folders: `pkgs/id/docs/`, `pkgs/mcpx/docs/` (see the
  documentation map in [`doc/README.md`](../../doc/README.md)).

## Architecture decision records (ADRs)

Decisions too small for a full dated design record — or that revise
one — are recorded as ADRs in [`adr/`](adr/README.md), using the
[ADR template](adr/TEMPLATE.md). Dated design records above remain the
record for major changes; an ADR that supersedes part of one links it,
and is linked back from the ADR index.

## Adding to this index

- A new dated architecture or design record in `doc/` gets one line in
  the matching section above, in the same PR that adds the record.
- A new ADR gets one line in the [ADR index](adr/README.md).
- Links in this file must resolve to files in the repository; that is
  the acceptance check for changes here.
