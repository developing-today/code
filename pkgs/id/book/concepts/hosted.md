# The hosted edition

ID is local-first: a native node holds its own stores and keys. The hosted edition is the same system offered as a service — a user's store and sync peer run in a Cloudflare Durable Object, bound to that user by a native binding ceremony, so a user without a running node still has a home for their signed objects. This chapter describes the hosted edition exactly as it stands, and the standing is the first fact:

**The hosted edition is not deployed.** The `hosted-worker` package in `pkgs/id/hosted-worker` is a local-only scaffold. Its `wrangler.toml` header states that the config is for `wrangler dev` and tests, that no account-level deployment, production binding or billable resource is authorized by the package, and that deployment and the real-account jurisdiction canary are gated on #2285 and #3403. Nothing in this chapter describes a running service. There is no production hosted URL, no production account, and no live user on this code.

## The binding ceremony is native

The worker does not authenticate users itself. Binding a hosted identity to an ID key happens natively, in the facade crate at `pkgs/id/src/hosted_binding.rs`, with companion facade modules `hosted_export` and `hosted_ingest`. The worker's front door consumes the result: an already-authenticated resolved context carried in the headers `X-Binding-Id` and `X-Jurisdiction`, plus `X-Iroh-Id`.

The front door (`src/worker.ts`, 73 lines) strips every client-supplied internal header, re-sets the context internally (`x-hosted-binding-id`, `x-hosted-jurisdiction`, `x-hosted-routed: restricted`, `x-hosted-iroh-id`), enforces the front-door body limit, and forwards to the per-user Durable Object through the jurisdiction-restricted namespace only. Objects are named by `binding_id` — 32 lowercase hex characters — never by email or provider subject. `src/routing.ts` is the only module that touches the raw `USERS` namespace, and there is no unrestricted fallback. The jurisdiction side of this fence is covered in [Residency, jurisdiction and boundaries](residency.md).

## Bindings

The worker's `wrangler.toml` (compatibility date `2025-06-01`, name `id-hosted-worker`) declares:

- `USERS` — a Durable Object namespace bound to the `UserDO` class, migration tag `v1`, with SQLite-backed classes (`new_sqlite_classes = ["UserDO"]`).
- `BLOBS` — an R2 bucket named `id-hosted-blobs`, holding content-addressed blob bytes (#3811 slice A, #9954). Local emulation only.
- `USER_CEILING_BYTES` — an optional string variable: the per-user storage ceiling. Absent or empty means no ceiling; a malformed value fails writes closed. The front door also reads it for the body limit; `MAX_BODY_BYTES` caps a request body at 100 MiB.
- `BRIDGE_WAITING_PER_AUTHOR`, `BRIDGE_WAITING_TOTAL`, `BRIDGE_WAITING_MAX_AGE_MS` — optional string variables bounding the bridge waiting pool. Absent means the native defaults in `bridge-store.ts`; malformed values fail bridge writes closed.

A separate test harness config (`wrangler.test.toml`, name `id-hosted-worker-test`) binds `TEST_USERS` to the same `UserDO` class, a bucket `id-hosted-blobs-test`, and `USER_CEILING_BYTES = "1073741824"`. It is test-only and is never an application entry point.

Locally, the worker fails closed on jurisdiction: local workerd does not implement jurisdiction restrictions, so every fully valid request receives 503 `jurisdiction_unavailable`, by design.

## What UserDO implements

The per-user object is `UserDO` in `src/user-do.ts`. Its schema version is 3, stored in SQLite tables `meta`, `kv` and `blobs`, plus the bridge schema (`bridge_events`, `bridge_chains`, `bridge_forks`, `bridge_proof_ids`, `bridge_proof_headers`, `bridge_waiting`, `bridge_waiting_by_dep`, `bridge_meta`) — the same table family as the native event store in [Events and the signed audit chain](events.md), under `bridge_` names.

Every route first runs the common guard: routing context present, runtime jurisdiction matches, and the persisted (binding_id, jurisdiction) pair — bound immutably by the first routed request — matches. The object exposes 15 method routes over 13 paths:

| Method and path             | Purpose                                                                                                             |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `GET /v1/usage`             | Usage, ceiling, and whether the user is at cap (`at_cap`)                                                           |
| `GET /v1/export`            | Full KV and blob index export; stays available at cap, by design                                                    |
| `GET /v1/state`             | binding_id, jurisdiction, schema_version, object name                                                               |
| `GET /v1/bridge/challenge`  | Issue a single-use ID-key proof nonce (bridge, #3818)                                                               |
| `GET /v1/bridge/heads`      | Bridge chain heads; a fresh ID-key proof is required per call                                                       |
| `POST /v1/bridge/push`      | Ingest a batch of signed events; outcomes are accepted, duplicate, fork, waiting, refused                           |
| `POST /v1/bridge/pull`      | Pull events beyond per-author positions; binary batch, with `x-bridge-more` and `x-bridge-heads` headers            |
| `GET /v1/bridge/export`     | Export sets for the native `build_export` (#13157); 502 naming the hash if R2 cannot produce a cited blob           |
| `WS /v1/bridge/ws`          | Bridge session; in-band CHALLENGE/AUTH, binary frames only                                                          |
| `PUT /v1/blobs`             | Store a content-addressed blob, keyed by SHA-256 hex; deduplicated across users, with a per-user reference row      |
| `GET /v1/blobs/{64-hex}`    | Fetch a referenced blob                                                                                             |
| `DELETE /v1/blobs/{64-hex}` | Drop this user's reference only; the R2 bytes are deliberately not deleted (garbage collection is the #9935 family) |
| `PUT /v1/kv/{key}`          | Per-user KV write; the key must match `[A-Za-z0-9._-]{1,128}`                                                       |
| `GET /v1/kv/{key}`          | KV read                                                                                                             |
| `DELETE /v1/kv/{key}`       | KV delete; available at cap                                                                                         |

Any other path or method returns 404 with `{"error":"not_found"}`. Bridge routes additionally fail closed when the binding record names no ID (Iroh root) key.

Read the table as a data plane, because that is what it is. The hosted object holds one bound user's KV, blob references, and a signed-object bridge store. As a sync peer it holds signed objects and fork evidence; it assigns no order, re-signs nothing, and serves no authoritative snapshot. Ordering and authority stay with the authors' chains, exactly as on a native node.

Also in the package, but not routed by `worker.ts`, are `src/provisioning.ts` and `provisioning-surface.ts` (a provisioning surface), `src/account-directory.ts`, `src/admission-gate.ts`, `src/export.ts`, and a currency gateway proof of concept under `src/currency/` (`gateway`, `ledger`, `relay`, `crypto`; #2298). The currency proof of concept is explicitly not the permanent currency service: it is an in-process ledger stub modelled on TigerBeetle, with gateway-signed balance snapshots and a UserDO relay reconciling by request-ID lookup. [Currency, tokens and metering](currency.md) covers it in place.

## What it does not yet do

The list of absences is part of the specification of the current state:

- No deployment exists. The gates are #2285 and #3403; until they clear, the package runs only under `wrangler dev` and its test harness.
- No jurisdiction canary has run against a real account, so the jurisdiction-restricted namespace behaviour is verified locally only in its fail-closed form (503 `jurisdiction_unavailable`).
- Blob garbage collection does not exist; deleting a blob drops one user's reference and leaves the R2 bytes in place (#9935 family).
- The hosted-worker README's own route list is stale: it describes the first slice (`GET /v1/state` and KV PUT/GET, with blobs, metering and caps as later steps) and was never revised. The code implements blobs, usage and metering, export, KV DELETE and the full bridge. This is survey disagreement WEB-9; the code, not the README, is the source for the table above.

## worker-poc: a completed proof, not a service

`pkgs/id/worker-poc` is a separate, finished proof of concept for #2271: run one example world unmodified in a Worker isolate and prove journal equality against the native wasmtime host. All of its acceptance boxes are checked (2026-11-01). Nothing in it is a production path.

Its worker (`src/worker.ts`) exposes a `WorldObject` Durable Object (binding `WORLD`, migration `v1-sqlite`, a CompiledWasm rule for the prebuilt guest) with three routes: `POST /step` taking `{participant, event}`, `POST /cold-start`, and `GET /state`. A SQLite `journal` table is the source of truth. The hosted loader (`src/manifest.ts`) refuses `portability=native` manifests and manifests declaring more than 64 MB of memory, up front.

A subfolder, `d1-journal-poc/`, is a second, local-only fit probe for D1: `GET /health`, `POST /append`, `GET /snapshot`, `GET /tail`, with journal header version 1 entries `created` and `caps_granted`, a placeholder database ID, and always `--local`. Its own note records that a D1 bookmark is not a stream cursor.

## Where the rest lives

The design for a deployed hosted edition — provisioning and SSO binding, per-user stores, storage contracts, self-hosted tier adapters and the media plane — is carried by the design chapter hosted and serverless editions (30 records in the survey's mapping). The shipped scaffold above is the ground that design stands on, and this chapter's claim is only the scaffold's: implemented locally, gated, and not running anywhere.
