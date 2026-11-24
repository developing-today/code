# Design: capabilities and providers

This chapter covers 44 design records: the capability model, ceilings, and the provider system that gives guest programs and shells their effects. It describes the system as designed in the programme's design archive. The final section states what runs today.

The provider model is tracked as #5686, with children that include #5900 (grants), #5902 (terminal and website host providers), #5904 (offline providers), #5905 (security model), #5898 (declaration model) and #5899 (discovery and negotiation). The first provider implementations are #1062, under Phase 12 (#534): #1061 is the provider registry, #1062 the first providers, #1063 the audit log, #1067 fair scheduling and quotas.

## The capability model

The core design record is `2026-11-08T09-15-00Z_design_id_capability_model` (2026-11-08): the capability model stated as requested sets versus discovered-used sets, a registry schema, and custom-compiled platforms. The distinction the record fixes is the chapter's spine: what a program asks for and what it actually uses are two different sets, and the system records both.

The derivation of the used set has two records, written a day apart as a spec and then a design: `2026-11-11T02-55-00Z_design_used-capability-derivation` (2026-11-11), the used-capability derivation tool spec, and `2026-11-11T08-50-24Z_design_used_capability_derivation` (2026-11-11), the design of the tool as a static call-graph scan producing the registry used-set (#5998).

The earliest record in the chapter is also the model's root: `2026-10-09T09-59-07Z_design_attenuated_capabilities` (2026-10-09), "Attenuated capabilities for world access". A capability in this design can be passed on only in a weaker form: delegation narrows, it never widens. `2026-11-01_design_id2028.md` (2026-11-01) sets the floor for the whole model: ID core v1, the app model and the minimum capabilities every app can assume.

Admission is part of the model, not an implementation detail: `2026-11-01_design_id2898.md` (2026-11-01) designs reconstruction-is-admission for `pkgs/id` (#2898) — an object or program is admitted by reconstructing and verifying it. The foundation chapter covers the same record from the graph side; see [Design: foundations — objects, graph and the file core](foundations.md).

## Ceilings

A ceiling is the upper bound on what a world, a program or a client may hold, set by an authority above it. The design records:

- `2026-10-16T11-00-00Z_feature_world_capability_ceilings` (2026-10-16) designs world capability ceilings.
- `2026-10-16T12-00-00Z_feature_world_cli_controls` (2026-10-16) designs the world operator CLI controls for capability ceilings and resource profiles — the ceiling is an operator surface, not a config file only.
- `2026-10-25T05-37-57Z_design_client_capability_providers` (2026-10-25) designs client-side capability providers under the ceiling model (F13.7, Phase 13), and `2026-11-01_design_id789.md` (2026-11-01) is the Phase 13 F13.7 design for the same subject (#789, with the rollout under #5532).
- `2026-11-17_runbook_id5532_f13-7_client_file_read_rollout.md` (2026-11-17) is the rollout runbook for `client.file.read`, the first client-side capability provider: the ceiling model's first concrete client provider is a file-read provider scoped by the shell, recorded again in [Design: worlds and the guest runtime](worlds-runtime.md) because the shell contract's conformance suite names it as the native shell's only declared provider.

Change and visibility are designed, not improvised. `2026-11-08T10-12-00Z_design_id_capability_diff_semantics` (2026-11-08) designs capability-change diff semantics: user-meaningful change classes and the changes that trigger re-approval. `2026-11-08T10-40-00Z_design_id_capability_observability` (2026-11-08) designs capability observability: who can see which capability facts, and where. `2026-11-08T01-10-24Z_design_id_capability_certification_exceptions` (2026-11-08) designs the capability-certification exception policy — exceptions exist, and they are a policy with records, not a bypass. `2026-11-23T07-06-00Z_design_id_capability_class_assignments` (2026-11-23) assigns capability classes for the post-#5999 registrations.

## The provider registry and provider families

Providers are the named implementations behind capabilities. The registry design starts at `2026-11-03T10-06-40Z_architecture_provider_registry_slice1` (2026-11-03), "F12.4 Provider Registry — Slice 1" (#1061). The declaration lifecycle the design gives a provider is filed, admitted, live, then suspended, superseded or stale, then tombstoned; the declaration records are `2026-11-11T00-18-00Z_decision_id5898_provider_declaration_model.md` (2026-11-11, manifest contents, signing, versioning, dynamic versus static capabilities; surveyed into the identity chapter) and the security model is `2026-11-09_decision_id5905_provider_security_model.md` (2026-11-09, identity, admission, impersonation, rogue providers, bridge compromise, capability confusion, ceiling composition; also surveyed into the identity chapter). Grants for providers are decided in `2026-11-09T00-16-56Z_decision_id5900_provider_capability_grants.md` (2026-11-09): grant form, sensitivity classes and approval flows (#5900).

The first provider family (#1062) has one design record per provider, all dated 2026-11-22:

- `2026-11-22_design_id1062-kv-provider.md` — the `kv` provider, first slice of #1062.
- `2026-11-22_design_id1062-blob-provider.md` — the `blob` provider, second slice.
- `2026-11-22_design_id1062-http-fetch-provider.md` — the `http.fetch` provider, third slice.
- `2026-11-22_design_id1062-notify-provider.md` — the `notify` provider, fourth slice.

Host-side instantiations of the #5686 model are decided in `2026-11-16T09-03-20Z_decision_id5902_terminal_website_host_providers.md` (2026-11-16): terminal and website host providers (#5902). Embedded instantiation is decided in `2026-11-08T09-10-00Z_design_embedded_providers_decision.md` (2026-11-08): engine and smartphone instantiation of the provider model. Offline behaviour is decided in `2026-11-08_decision_id5904_offline_degraded_providers.md` (2026-11-08): offline and degraded operation for capability providers (#5904).

The remaining provider families in the design:

- `2026-11-04T06-40-00Z_feature_loc-capability.md` (2026-11-04) designs the `loc.*` location capabilities, with an accuracy ceiling and per-use consent as part of the capability itself.
- `2026-11-20T07-30-11Z_feature_speech-capabilities.md` (2026-11-20) designs the `speech.*` conversational audio capabilities.
- `2026-11-09T05-00-00Z_feature_mqtt_device_bridge_id_device_v1` (2026-11-09) designs the `id.device.v1` MQTT device bridge, schema and proof of concept (#2343); the speech thread pairs with #2344 in the programme's early trackers.
- `2026-10-16T18-00-00Z_feature_records_readback` (2026-10-16) designs records readback as a capability: `records.read`.
- `2026-10-16T16-00-00Z_feature_roc_cap` (2026-10-16), RocCap, is the guest-side bookkeeping library for the same protocol; see [Design: worlds and the guest runtime](worlds-runtime.md).

Discovery and negotiation between a consumer and a provider are designed in `2026-11-08T09-10-14Z_design_id5899_provider_discovery_negotiation` (2026-11-08), surveyed into the sync chapter because negotiation is a wire concern; see [Design: sync, replication and FileOps](sync-fileops.md). Routing across providers is supplemented by `2026-11-13T09-26-00Z_design_pool_effort_system` (2026-11-13): per-capability routing, failover, and pools treated as models.

## Audit, admission and quotas

`2026-11-01T20-45-00Z_feature_admin_audit` (2026-11-01) designs F12.6 slice 1: administrative and provider-use audit (#1063's audit thread). Provider use is auditable by design; the audit record is part of the grant, not a log added later.

Fair use has two records under #1067, both dated 2026-11-22/23: `2026-11-22_design_id1067-admission.md` (2026-11-22), guest-burn admission, the second slice of #1067, and `2026-11-23_design_id1067-account-quotas.md` (2026-11-23), per-account quotas, the ownership slice of #1067.

## Taxonomy, sensitivity and capability-bearing claims

- `2026-10-29T14-05-00Z_investigation_capability_taxonomy` (2026-10-29) is the capability taxonomy v0 investigation: interface specialization across web, ESP32/MQTT and text/audio surfaces.
- `2026-11-09T06-25-00Z_research_id5989_prior_art_capability_systems` (2026-11-09) records the prior art: capability systems, object-capability design, Spritely Goblins and OCapN, and CRDT shared worlds.
- `2026-11-11_decision_id5817_sensitivity_classification.md` (2026-11-11) decides the sensitivity classification model for ID table queries (#5817).
- `2026-11-08_decision_id5909_sql_pgq_capability.md` (2026-11-08) decides the SQL/PGQ capability boundary for ID triples: graph query access is a capability with a boundary, not a store login.
- `2026-11-08T17-12-39Z_design_id_package_import_policy` (2026-11-08) designs pure-package imports and package classes across capability tiers.
- `2026-11-08_design_id6015.md` (2026-11-08) designs land/parcel capability domains: per-parcel policy granularity without tokenized land.
- `2026-11-01_design_id786.md` (2026-11-01) designs F13.4 presentation capability tiers with in-band terminal detection (#786): presentation itself is tiered as a capability.

Three claim systems are capability-bearing in the design and are recorded here. Tag claims: `2026-11-06_decision_id_tagging_triples.md` (2026-11-06) decides ID tag claims as atomic-triple projections, and `2026-11-08T06-25-00Z_design_tag_scope` (2026-11-08) designs tag scope, local-only versus shared claims (#1793). Licensing: `2026-11-05T17-24-09Z_design_id_license_detection_pipeline` (2026-11-05) designs the upload-time license detection pipeline; `2026-11-06_design_id5985.md` (2026-11-06) designs the license combination checker (combination events, v1 incompatibility rules, override versus exception-request flows, audit records); `2026-11-10_design_id5977_license_taxonomy.md` (2026-11-10) designs the license taxonomy and classifier: the enumerated set, the warning ladder, and the tag format.

## What exists today

Grounded in the code surveys, not the design records:

- The provider declaration lifecycle exists in code: `crates/id-caps/src/declaration.rs` implements the states filed, admitted, live, suspended, superseded, stale and tombstoned, with signing domains `id.provider-manifest.v1` and `id.provider-withdrawal.v1`. Suspension in the shipped code is registry state; the provider's contents do not change.
- The ceiling model exists in code at the MCP bridge policy point: `crates/id-caps/src/mcp_policy.rs` enforces a capability ceiling (tools, worlds, object classes, program allowlist), charges quota before execution, and writes exactly one hash-chained audit event per action, refusals included. That is the design's ceiling-plus-audit rule running in one place; it is a library policy point, not a deployed MCP server.
- The configuration contract carries ceilings as hard invariants: the `id-config` crate's document requires a `ceilings` object (`max_hops`, `max_bytes`, `max_time_ms`, `max_fan_out`, each at least 1), and scoped overrides may only narrow a ceiling; a widening override is rejected with `CeilingWidened`, never clamped.
- The first client provider rollout is real as a runbook and a shell-contract entry: `client.file.read` is the only provider the shell conformance suite declares, for the native shell.
- Still design only, with no shipped counterpart verified in the surveys: the four #1062 provider records (`kv`, `blob`, `http.fetch`, `notify`) as a complete provider family, the `loc.*` per-use-consent flow, and the `speech.*` family. The surveys verify the registry and ceiling machinery; the full provider catalogue remains design. This point is stated as unverified for each individual provider beyond the records cited.
