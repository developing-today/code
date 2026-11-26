# ID service, Workers edition: gap analysis (#9080)

Component B of the hosted editions epic (#9078). This record maps every element of the serverless ID line (#2046 and its implementation #5524, the storage design #5681 and its children, the hosted Worker API #3811, provisioning and binding #2852) to what is on main, names what is missing, answers the tenancy question against component E (#9083), and lists the V1 surfaces. New issues are filed only for gaps nothing else owns.

Checked against main at `8ca969054` on 2026-11-26. The hosted worker is `pkgs/id/hosted-worker/` (HW below).

## Summary

The hosted ID line has most of its parts built and tested, but it is not a service a per-user instance can run against yet:

- nothing authenticates a caller into the worker;
- most tenancy and provisioning modules are libraries that no route serves;
- the store is Durable Object SQLite, and none of the #5681 storage contract is implemented on it;
- the worker is not deployed (`wrangler.toml` says LOCAL ONLY and the worker is not in the release workflow).

Eight gaps had no owner and are filed as children of #9080: #15479 to #15486.

## Covered

| Element | Source | On main |
| --- | --- | --- |
| Front door, binding_id routing, fail-closed jurisdiction | #2046, #2283 | HW `src/worker.ts`, `src/routing.ts` (PR #5286) |
| Lazy per-user Durable Object with versioned SQLite bootstrap | #3811 | HW `src/user-do.ts` (PR #5286) |
| Content-addressed R2 blobs and reference table | #3811 (#9954) | PR #10129 |
| Metering, storage ceiling, read-only and export at cap | #3811 (#9955) | PR #10132; `GET /v1/usage`, `GET /v1/export` |
| Client-side-encryption flag on blobs | #3811 | PR #15072 |
| Account and binding ceremony, delayed rebind | #2852 slices 1 and 2 | `pkgs/id/src/hosted_binding.rs` (PRs #4854, #4885) |
| Export format, verification and production from a DO | #2852 slice 4 (#13157) | `src/hosted_export.rs` (PR #9396), HW `src/export.ts` (PRs #13212, #13312) |
| Import ingest | #13158 | `src/hosted_ingest.rs` (PR #13201) |
| Signed-object bridge, hosted and native sides | #3818 | HW `bridge-*.ts`, `event.ts`, `range.ts` (PR #12911); `crates/id-sync/src/bridge.rs`, `bridge_http.rs` |
| Provisioning gate and enablement point | #3403 | HW `provisioning.ts`, `provisioning-surface.ts`, `enablement.ts` (PRs #12985, #13162, #13753) |
| Jurisdiction migration and world partition libraries | #3404, #3898 | `src/hosted_migration.rs`, `hosted_freeze.rs`, `hosted_world_partition.rs`; HW `migration-control.ts` |
| Tenancy schema, TenantOpsDO, credentials, identity links | E1, E2, E4, H2 | HW `tenancy/migrations/`, `tenant-ops.ts`, `credentials.ts`, `identity-link.ts` (PRs #13042, #14392, #14480, #14418) |
| Storage decisions | #5681, #5863, #5866, #5867, #5868 | decision records only (see gap 5) |

## Gaps

| # | Gap | Owner |
| --- | --- | --- |
| 1 | The front door trusts `X-Binding-Id` and `X-Jurisdiction` from an internal caller that does not exist; no session or OIDC check; the binding ceremony has no HTTP caller. | #15479 (filed) |
| 2 | Three account and binding registries (`migrations/0001_hosted_accounts.sql`, the tenancy tables, `hosted_binding.rs`); `HOSTED_DIRECTORY` is unbound, so the check is skipped. | #15480 (filed) |
| 3 | `worker.ts` routes only to `UserDO`; TenantOpsDO, credentials, identity links, provisioning surface, enablement, admission gate and migration control are not served. | #15481 (filed) for the entry point; sync ingress is E3 #12798, the public plane E5 #12800 |
| 4 | No record says which tenant data lives in DO SQLite, the per-user Turso DB or D1. | #15482 (filed, design) |
| 5 | The #5863 contract (read-at-watermark, typed refusals), the #5868 budget ladder and the #5867 backend manifest are not implemented on the hosted store. | #15483 (filed); the Turso adapter is E6 #12801 |
| 6 | The bridge WebSocket uses `server.accept()`; #3811 required hibernating sockets. | #15484 (filed) |
| 7 | Requests are counted but never refused; #2046 section 6 requires a retryable refusal at the cap. | #15485 (filed) |
| 8 | No `id` command uses the bridge client, so a user cannot push to or pull from a hosted instance. | #15486 (filed) |
| 9 | Not deployed; no closed beta (#2046 phase 6). `wrangler.toml` still cites #2285 and #3403 as gates, both closed. | #5524 (open; this is its remaining scope) |
| 10 | Jurisdiction-aware per-user stores, blob isolation, migration. | #5523, #3401, #3402, #3404 |
| 11 | Hosted worlds partitioned by jurisdiction. | #5528, #3898 |
| 12 | Sync through SyncCoord on `/id-sync/1` as #9083 section 5 describes; the landed path is the HTTPS and WebSocket bridge. | E3 #12798; peer-auth conformance H5 #14263 |
| 13 | Public plane and F publication and GC. | #12800, #12806, #12807, #12808 |
| 14 | TypeScript transports and WASM scoping. | #9085 |

The browser IndexedDB store (bounded profile adopted under #9082) is a browser product question, not part of the hosted ID service, and is left out.

## Tenancy (question 2)

#9083 answered this in its own record (`doc/2026-11-17T16-55-00Z_design_hosted-tenancy-architecture/`, section 9): it builds on #3811 and neither wraps nor supersedes it. The decision here follows that record:

- #3811's per-user Durable Object is the tenant data-plane layer under component E. `UserDO` is E's TenantDO, one per tenant, keyed by `binding_id`.
- Component E adds the control plane above it: the tenancy database, TenantOpsDO, SyncCoord, PublishIndex, and the per-user Turso database.
- #3811's metering stays authoritative per tenant; the tenancy database holds the rollup.

What neither record decides is how a tenant's data splits between the DO's SQLite and the per-user Turso database. That is gap 4 (#15482). It also has to settle #14866, which proposes Workers plus D1 for data planes and may cut across #9083's per-user Turso choice.

## V1 surfaces (question 3)

| Surface | Hosted V1 | Today |
| --- | --- | --- |
| Signed events and heads (sync) | yes | served through the bridge as opaque, chain- and fork-checked objects |
| KV and blobs | yes | served (`/v1/kv`, `/v1/blobs`) |
| Usage and export | yes | served (`/v1/usage`, `/v1/export`) |
| Identity: signup, binding, rebind | yes | library only (gaps 1 to 3) |
| Groups and memberships | no, native only for V1 | not served; `bridge-store.ts` leaves membership resolution out |
| Capabilities | no, native only for V1 | not served |
| Directory | no, native only for V1 | not served |
| Worlds | no, native only for V1 | native, partitioned under #3898 |

Groups, capabilities, directory and worlds stay native for V1 because each needs group resolution or a live actor that the bridge store does not carry; the hosted instance syncs their signed events, and a native peer evaluates them. Moving any of them into the worker is a later, separately filed decision.

## Housekeeping found

- #9080's body lists #2852 slices 3 and 4 and #3811 as open work; both issues are closed.
- #5524 stays open as an acceptance tracker; with #2852 closed, its remaining scope is gap 9 (deployment and beta).
- #14260 is open although its PR #14418 merged.
