# Design: hosted and serverless editions

This chapter covers 30 design records: the design for running ID hosted — a Cloudflare Workers and Durable Object edition, per-user stores, serverless storage contracts, and the adapters that let foreign stores and self-hosted tiers stand behind the same interfaces. It describes the system as designed in the programme's design archive. The final section states what runs today.

The programme trackers are #9078 (the hosted-editions programme), #2046 (the serverless edition, a sub-epic of #2056), #5681 (serverless storage), #5672 (storage decomposition) and #6283 (hosted worlds). The #9078 intake record covers components A (PM service), C (fleet tool), E (hosted tenancy, #9083), F (content-addressed media, #9084) and I (the self-hosted tier, #9087); components A and C are not ID, and this chapter covers the ID service and components E, F and I.

## Fitting ID onto Cloudflare Workers

The fit investigation is `2026-10-28T23-34-14Z_investigation_cloudflare_workers_fit` (2026-10-28): fitting the ID platform onto Cloudflare Workers. The edition design itself is `2026-11-01_design_id2046.md` (2026-11-01): the ID serverless edition v1 on Cloudflare Workers (#2046). An earlier, narrower record, `2026-10-14T10-18-19Z_design_p7_2_cloudflare_workers.md` (2026-10-14), designs the P7.2 Cloudflare Workers pkarr option — the first point where the programme put a piece of ID's discovery layer on Workers.

The intake record for the whole programme is `2026-11-12T23-04-00Z_design_hosted-editions-workers` (2026-11-12): hosted editions as workable Cloudflare Workers equivalents for the PM service, the ID service and the fleet tool. For ID, the design question the intake poses is equivalence: a hosted ID service that native nodes treat as a peer, not a reduced web front end.

Two investigations define the seams. `2026-10-29T11-27-00Z_investigation_bridge_semantics_hosted_native` (2026-10-29) investigates bridge semantics: the hosted (HTTPS/WebSocket) edition syncing with native Iroh nodes. The bridge that investigation leads to is recorded as built in `2026-11-23T05-00-00Z_design_bridge_v1_implementation` (2026-11-23): the hosted signed-object bridge v1, implementation record for #3818. `2026-10-29T11-28-18Z_investigation_key_recovery_hosted` (2026-10-29) investigates key recovery for hosted ID users: Google SSO loss versus the ID keypair — the hosted account credential and the identity key are different objects, and losing the first must not lose the second. Provisioning is designed in `2026-10-30T22-45-00Z_design_hosted_provisioning_sso_binding.md` (2026-10-30): hosted provisioning plus SSO-to-ID binding, the ceremony that binds a hosted account to an ID key.

Hosted worlds are designed in `2026-11-16_design_id6283_hosted_worlds_v1.md` (2026-11-16): hosted worlds and always-on hosting, v1 (#6283). The residency rules a hosted world lives under are in [Design: residency, jurisdiction and boundaries](residency-jurisdiction.md).

## Serverless storage: the #5681 synthesis and its contracts

The parent synthesis for serverless storage is `2026-11-10_decision_id5681_serverless_storage.md` (2026-11-10): the decision record for the serverless edition storage programme (#5681). The contracts under it:

- `2026-11-17_decision_id5863_storage_contract.md` (2026-11-17) decides the serverless storage contract: journal, world state and indexes as three named stores with one contract each.
- `2026-11-17_decision_id5864_d1_journal_fit.md` (2026-11-17) designs the D1 journal fit: ordering, sessions and replication of the signed journal against Cloudflare D1.
- `2026-11-10_design_id5865.md` (2026-11-10) designs the Turso/libSQL journal fit: how libSQL replication composes with the signed journal.
- `2026-11-08_decision_id5868_cost_limits_guardrails.md` (2026-11-08) decides cost and limits guardrails for serverless storage.
- `2026-11-08_design_id5866.md` (2026-11-08) designs local-dev parity for the serverless edition: the local development form runs the same contracts as the deployed form.
- `2026-11-08_design_id5869.md` (2026-11-08) designs backup, recovery and exit for serverless storage: a hosted user can leave with their data, as a designed path.

The staged store designs under #5672 decompose the store by role. `2026-11-08_design_id5824.md` (2026-11-08) is Stage A: a Redis hot store with a regular store as the system of record. `2026-11-11T00-16-00Z_design_id5807_redis_sqlite_export` (2026-11-11) designs the Redis-to-SQLite export and the query-Redis-get-SQLite path (#5807). Backend selection and migration between these stages is decided in `2026-11-12_decision_id5867_backend_migration.md` (2026-11-12), the backend selection and migration decision for #5867.

Hosted tenancy is designed in `2026-11-17T16-55-00Z_design_hosted-tenancy-architecture` (2026-11-17), component E of #9078: per-user Turso databases plus a central tenancy database (#9083's architecture record).

## Adapters: foreign stores, browsers and the self-hosted tier

- `2026-11-08_design_id5803.md` (2026-11-08) designs the shared foreign data-store adapter contract for ID: one contract that foreign stores implement to stand behind ID's store ports (the port discipline itself is in [Design: foundations — objects, graph and the file core](foundations.md)).
- `2026-11-08_design_id5804.md` (2026-11-08) designs the IndexedDB adapter: origin-scoped addressing, live versus materialized reads, structured-clone type mapping, and storage limits and eviction — the browser's store as an ID store.
- `2026-11-21T21-40-00Z_design_self_hosted-tier-adapters` (2026-11-21) designs the self-hosted tier (component I, #9087): storage, transport and placement adapter design, so a self-hosted node and the hosted edition differ by adapters, not by protocol.
- `2026-11-08T23-22-53Z_design_id_adapter-conformance` (2026-11-08) designs the adapter conformance harness and interop matrix CI: an adapter is admitted by passing the harness, and the matrix is the record of which adapters pass.
- `2026-11-14T13-10-00Z_design_id5812_adapter_record.md` (2026-11-14) is the adapter decision record for WebRTC, TCP/TLS and local IPC (#5812).
- `2026-11-09_design_id5815_security_binding.md` (2026-11-09) reviews the security binding across adapters and relays.

Transport into the hosted edition has its own records: `2026-11-05T15-10-00Z_design_cloudflare_workers_rendezvous_relay.md` (2026-11-05) designs the Cloudflare Workers rendezvous/relay component (#5794), and `2026-11-08T09-45-00Z_design_id5811_wss_https_adapter.md` (2026-11-08) designs the WebSocket/WSS plus HTTPS fallback adapter for browsers and restrictive networks. The sync and fallback design behind both is in [Design: sync, replication and FileOps](sync-fileops.md). The media plane is designed in `2026-11-23T03-04-00Z_design_content-addressed-media-data-plane` (2026-11-23): the content-addressed media and data plane, reference-based access and garbage collection (component F, #9084).

One record is surveyed here at the chapter's edge: `2026-11-09_design_id5783.md` (2026-11-09) designs the ESP32 hardware platform line — boards, provisioning, firmware lifecycle, over-the-air updates and power. Its book home in the argument is the shells and hardware chapter, [Design: shells and client surfaces](shells-surfaces.md).

## The federation cluster behind the hosted edition

The hosted edition federates, and the federation design cluster is #9117/#9141/#9144 inside #9129–#9148. The three anchor records are surveyed into neighbouring chapters and cross-linked here because the hosted edition is their largest consumer: the `id://` URI scheme (#9117) in [Design: foundations — objects, graph and the file core](foundations.md), peer trust classes (#9141) in [Design: identity, directory, groups and delegation](identity-directory-groups.md), and federated query fan-out (#9144, `2026-11-17T15-40-00Z_design_id_federated_query_fanout`, 2026-11-17) in the shells chapter. A hosted node in this design is one federation peer among others, addressed by `id://`, trusted by class, and queried by fan-out under the fan-out budget the configuration contract enforces.

## The hosted-worker scaffold as the first step

The design's first concrete step is the hosted-worker scaffold in `pkgs/id/hosted-worker`. It is a scaffold: the front door, the per-user Durable Object, the blob store binding and the signed-object bridge exist as code and tests, under a local-only configuration. The scaffold's own config header states the limit in plain terms: the config is for local development and tests, and no account-level deployment, production binding or billable resource is authorized by the package. Deployment and the real-account jurisdiction canary are gated on #2285 and #3403 — the second gate is part of the jurisdiction chain covered in [Design: residency, jurisdiction and boundaries](residency-jurisdiction.md).

## What exists today

Grounded in the web and configuration survey, not the design records:

- The hosted worker exists as a local-only scaffold, not a deployed service. Its front door consumes an already-authenticated binding context (the binding ceremony itself is native code, `pkgs/id/src/hosted_binding.rs`), strips client-supplied internal headers, re-sets them internally, enforces a front-door body limit (maximum body 100 MiB, from the `USER_CEILING_BYTES` binding with a 100 MiB cap), and forwards to the per-user Durable Object through the jurisdiction-restricted namespace only.
- The per-user Durable Object (`UserDO`, SQLite schema version 3) implements 15 method routes: usage, export, state, blob put/get/delete (content-addressed by SHA-256 hex, deduplicated across users with per-user reference rows in R2), KV put/get/delete (keys match `[A-Za-z0-9._-]{1,128}`), and the bridge family — challenge, heads, push, pull, export and a WebSocket session. The bridge store holds signed objects and fork evidence; it assigns no order, re-signs nothing, and serves no authoritative snapshot. Bridge push outcomes are accepted, duplicate, fork, waiting and refused.
- The test harness binding fixes one concrete ceiling: `USER_CEILING_BYTES = "1073741824"` (1 GiB) in the test configuration. In the application configuration the variable is optional: absent means no ceiling, and a malformed value fails writes closed.
- The world-on-Worker proof exists as `worker-poc`: one example world runs unmodified in a Worker isolate with journal equality against the native wasmtime host proven (2026-11-01). The hosted loader refuses manifests with `portability=native` and manifests declaring more than 64 MB of memory up front. `worker-poc` is a proof of concept for #2271, not a production path.
- The native side of the edition ships in the root crate: `hosted_binding`, `hosted_export` and `hosted_ingest` modules, plus a foreign libSQL adapter module (`foreign_libsql`) that resolves credential references of the form `env:<VAR>` — the shipped beginning of the foreign-store adapter design.
- Still design only: the deployed hosted edition itself, hosted tenancy on per-user Turso databases as a running service, the Redis hot-store stage, and the IndexedDB adapter as a shipped store. The scaffold, the bridge and the proofs of concept are real; the edition as a service is not deployed, and no record or survey in the book's sources states otherwise.
