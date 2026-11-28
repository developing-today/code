# Data stores and schemas

Source: https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-store/src/event_types.rs, https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-events/src/event_store/mod.rs, https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-store/src/tags.rs, https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-sync/src/fileops.rs, https://github.com/developing-today/code/blob/main/pkgs/id/test-vectors/schema.json

This chapter is the inventory of named, versioned formats a reader, operator or implementer will meet on disk or on the wire: about 35 of them. Paths are relative to the data directory unless stated; the data directory itself is covered in [Configuration and environment](configuration.md).

## The blob and name store (id-store)

| Item           | Value                                                                                |
| -------------- | ------------------------------------------------------------------------------------ |
| Implementation | iroh-blobs `FsStore`: a redb database plus data files.                               |
| Path           | `.iroh-store/`; the iroh-docs store lives under `.iroh-store/docs/`.                 |
| Open rule      | Single-opener: the redb file lock admits one process.                                |
| Ephemeral form | An in-memory variant backs `id serve --ephemeral` and the other `--ephemeral` paths. |
| Node key       | The `.iroh-key` file family sits alongside the store, in the data directory.         |

A "file" in ID is a name (an iroh-blobs tag) pointing at an immutable blob hash, plus TagStore metadata. That model is implemented once, in FileOps (below), and shared by the CLI, the REPL, the meta protocol and the web layer.

## The tag store and registry

| Item             | Value                                                                                                                                                  |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Backing          | iroh-docs α/Ω namespace pairs: a global namespace, a node-private namespace, and custom paired or single namespaces.                                   |
| Registry file    | `.iroh-meta/registry.json`                                                                                                                             |
| Registry version | `REGISTRY_VERSION = 1` (`crates/id-store/src/tags.rs`). The file records the namespaces and one-time migration completion.                             |
| Values           | Arbitrary bytes. Display and wire forms use lossy UTF-8, plus `value_b64` where binary fidelity is required; see [WebSocket protocols](websockets.md). |

Other named hash domains in `id-store`: `id.name-candidate/v1`, `id.name-curation/v1`, `id.location-tombstone/v1`, and `id.local-claim.v1` (the tag-claims hash domain).

## The graph index

| Item         | Value                                                      |
| ------------ | ---------------------------------------------------------- |
| Backing      | redb.                                                      |
| Tables       | `graph_spo`, `graph_pos`, `graph_osp`, `graph_index_meta`. |
| Sort profile | `address-asc/v1`.                                          |

## Sidecar snapshots

| Item           | Value                                       |
| -------------- | ------------------------------------------- |
| Head file      | `hash.metadata`.                            |
| Snapshot files | `hash.metadata.<hex digest of content id>`. |
| Record format  | CBOR records with `RECORD_VERSION = 1`.     |
| Head domain    | `id.sidecar-head/v1`.                       |

## The event store (id-events)

| Item          | Value                                                                                                                                                                    |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| File          | `events.redb`, in the event-store directory (CLI `--event-store`, default `events` in the data directory). The temporary write name is `events.redb.tmp`.                |
| Format marker | **`id.events.store.v9`**, stored in the `meta` table. `id.events.store.v8` is also accepted: a v8 store is a v9 store with no `bodies` table and nothing else different. |
| Key encoding  | Big-endian, so one author's events sort in sequence order.                                                                                                               |
| Chain rows    | A fixed 41-byte encoding: next sequence (big-endian `u64`) ‖ head hash (32 bytes) ‖ frozen flag (`0`/`1`).                                                               |

### Tables: 17

```text
events            hashes            chains            forks
groups (multimap) proof_ids         proof_headers     waiting
waiting_by_dep (multimap)           meta              blocks
writer_set_bodies actions           device_ops        applied
bodies            tag_fold
```

The `blocks` table holds the operator block list. It is data, not configuration: no server-config key writes it (see [Configuration and environment](configuration.md)).

### The event wire format

Every event is a p2panda-core `Header<ChainExt>` in **canonical CBOR** (`crates/id-store/src/chain.rs`):

| Field        | Rule                             |
| ------------ | -------------------------------- |
| Domain tag   | **`id.events.v1`**               |
| Operation ID | BLAKE3 over the received bytes.  |
| Signatures   | `verify_strict`.                 |
| `ChainExt`   | `{kind, group, after}`.          |
| `after`      | At most 32 hashes (`MAX_AFTER`). |
| `kind`       | 1–64 bytes, NFC-normalised.      |

Ingest outcomes are `accepted`, `duplicate`, `fork`, `waiting` and `refused` — the same five outcomes the hosted bridge returns in [The hosted Worker API](hosted-worker.md).

### Event-kind constants

About 25 `kind` values are named as constants across `id-store`, `id-events` and `id-core`:

```text
chat.message.v1        chat.anchor.v1         document.put
document.remove        media.put              media.remove
note.create            note.revise            note.link
tag.set                tag.clear              session.checkpoint
identity               issuer.rotation        device.add
device.revoke          attestation.genesis    attestation.claim
attestation.witness_decision                 attestation.objection
idset.writer           idset.rescind          chained.message
```

In addition, the group operation kinds are carried by `id_events::groups` — group create, member and role operations, validated in the event store's resolve and ingest paths. The group commands that produce them are in [Groups, documents, claims and keys](cli-groups-identity.md).

## The directory journal (id-directory)

| Item         | Value                                                                                                                                                                                          |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| File         | `directory.jsonl`, in the server-config directory.                                                                                                                                             |
| Format       | One JSON `DirectoryEntry` per line. Serde-tagged on the field `entry`, snake_case. Replayed through the same `apply` path as live changes, so replay and live writes cannot diverge in effect. |
| Secrets rule | Account secrets enter only as SHA-256 digests. The account key domain is `id-account-v1`.                                                                                                      |

### DirectoryEntry kinds: 17

```text
account_signed_up       account_keyed           account_verified
account_key_added       account_key_removed     account_email_confirmed
account_email_removed   group_created           group_updated
group_visibility        group_deleted           member_set
tier_set                friend_requested        friend_accepted
friend_removed          friend_removal_noted
```

Supporting enums in the same records: `Member`, tagged on `kind`, with variants `account` and `group`; `Level` with variants `access`, `read`, `write`, `manage`, `moderator`, `admin`; `Tier` with variants `anonymous`, `registered`, `verified`.

### Keyset files

| File                      | Content                                    |
| ------------------------- | ------------------------------------------ |
| `.id-keyset.jsonl`        | JSONL key records for the server identity. |
| `.id-keyset-client.jsonl` | JSONL key records for the client identity. |

## Signed artifacts

Envelope wire types live in `id-sync::artifact` and are re-exported by `id-directory`.

| Item                 | Value                                                                                                                                   |
| -------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| Kinds: 4             | `friend_remove`, `home_declared`, `document`, `tag`. Their wire-name constants are `FRIEND_REMOVE`, `HOME_DECLARED`, `DOCUMENT`, `TAG`. |
| Signed-bytes profile | **`id-artifact-serde-json/v1`** (the `LEGACY_PROFILE` constant).                                                                        |
| Signed body          | `SignedBody{author, seq, prev, kind, body, after}` serialised as compact `serde_json`. The `after` field is omitted when empty.         |
| Hash                 | SHA-256, hex.                                                                                                                           |
| Signature            | Ed25519, hex.                                                                                                                           |

The profile is pinned by a conformance vector file; see "Test vectors" below.

## Listing, suspension and delisting records

No listing, suspension or delisting record kind exists in `crates/id-directory`. That family lives in three other places:

| Location                                                     | Format                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| ------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/id-sync/src/registry.rs`                             | Signed registry lists: a canonical-CBOR body with a registry-key Ed25519 signature. Entries are `RegistryEntry{announcement, included_at, registry_seq}`, where `announcement` is verbatim and world-key-signed. Delistings are `Tombstone{world_id, removed_at, reason, registry_seq}`; `reason` is at most 200 characters. The policy statement is at most 2000 characters. Unlisted worlds are refused at inclusion and skipped on import. |
| `crates/id-caps/src/declaration.rs`                          | Provider declaration lifecycle: `Filed → Admitted → Live → Suspended / Superseded / Stale → Tombstoned`. Suspension is registry state; the provider's contents are unchanged. Signing domains: `id.provider-manifest.v1`, `id.provider-withdrawal.v1`, `id.host-attested-provider:`.                                                                                                                                                          |
| `crates/id-worlds/src/program_gallery_admission.rs`          | Gallery listing decisions and transparency entries (removal, refusal, reinstatement) behind the routes in [The web UI: pages and HTTP API](web-ui.md).                                                                                                                                                                                                                                                                                        |
| `crates/id-sync/src/federation/slices/b02_key_revocation.rs` | Key suspension records: operator maintenance suspension, bounded or indefinite.                                                                                                                                                                                                                                                                                                                                                               |

## FileOps and the archive naming rule

FileOps (`crates/id-sync/src/fileops.rs`) is the single put/rename/copy/delete implementation for the CLI, the REPL, the meta protocol and the web UI.

### Archive naming

When a rename or replace archives the old file, the archive name is:

```text
{name}.archive.{unix-ts}
```

- On a same-second collision, the suffix is bumped — `{ts+1}`, then `{ts+2}`, and so on — until the name is free. The claim happens under an in-process lock.
- The shape `{stem}.archive.{digits}`, where the digits fit a `u64`, is a **reserved namespace at the writer**: such names are refused as user files, with the error `ReservedArchiveName`. Non-numeric lookalikes such as `a.archive.notes` stay valid user names.

### Archive and identity metadata keys

| Key                    | Placed on     | Meaning                                      |
| ---------------------- | ------------- | -------------------------------------------- |
| `archive.rename`       | —             | Records a rename archive relationship.       |
| `archive.replace`      | The live name | Names the archives the live name owns.       |
| `archive.owner`        | The archive   | Names the archive's owner.                   |
| `name`, `file`, `path` | The file      | Identity keys, refreshed on rename and copy. |
| `origin`               | The file      | Stamped at first write.                      |

Archives are dropped only with their owner. There is no per-archive delete.

### Delete semantics

`FileOps::delete` is a **hard** delete of the name, its metadata and its archives. The blob bytes survive until garbage collection. The web layer's soft-delete/restore pair (`/api/delete`, `/api/restore`) is built on top of FileOps and is not FileOps behaviour.

## Other id-sync formats

| Format                       | Value                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| ---------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Envelope ALPN                | `/id-envelope/1`                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| Record-store envelope domain | `id-sync/record-store-envelope/v1`                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Bridge authentication domain | `id.bridge-auth.v1` (the hosted bridge in [The hosted Worker API](hosted-worker.md))                                                                                                                                                                                                                                                                                                                                                                              |
| Federation record store      | `federation/record_store.rs`, with `RecordStatus`, and slices B01–B09, B15–B24, B28, B31, B32 covering key lifecycle and revocation, the replay guard, signature suites, signer binding, rejection events, conflict reports, sync scope, anti-entropy, peer priority, fanout budget, query grants and privacy, signed receipts, request identity, result dedup, loop inform, routing state, topology probe, takeover response, peer classes, and the ingest gate. |

## The tag registry

Covered above with the tag store: `.iroh-meta/registry.json`, format **`registry.json` v1** (`REGISTRY_VERSION = 1`), recording namespaces and one-time migration completion.

## Hosted Durable Object SQLite schema

The hosted Worker's `UserDO` (see [The hosted Worker API](hosted-worker.md)) uses SQLite with **`SCHEMA_VERSION = 3`**:

| Tables                                                                                                                                                 | Group                                                                                                |
| ------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------- |
| `meta`, `kv`, `blobs`                                                                                                                                  | Core object state: metadata, per-user KV, blob references.                                           |
| `bridge_events`, `bridge_chains`, `bridge_forks`, `bridge_proof_ids`, `bridge_proof_headers`, `bridge_waiting`, `bridge_waiting_by_dep`, `bridge_meta` | The bridge store: the signed-object sync peer state, mirroring the native event store's table roles. |

Two related proof-of-concept schemas, neither a production format:

| Schema                           | Value                                                                                            |
| -------------------------------- | ------------------------------------------------------------------------------------------------ |
| worker-poc `WorldObject` journal | An unversioned SQLite `journal` table; the source of truth for that PoC.                         |
| d1-journal-poc journal header    | Version `1`; entries include `created` and `caps_granted`. A D1 bookmark is not a stream cursor. |

## Program bundles and world formats

| Format                         | Magic / version                                                      |
| ------------------------------ | -------------------------------------------------------------------- |
| Program bundle (`.idpb` files) | Magic line `developing-today/id-program-bundle/v1` followed by `\n`. |
| World snapshot                 | Magic `IDWS`.                                                        |
| World replication frame        | Magic `id-world-replication/v1` followed by `\n`.                    |
| World module signature domain  | `developing-today/id-world-module/v1` followed by a NUL byte.        |
| Program-registry ALPN          | `/id-program-registry/1`.                                            |

## Keys, invites and presence

| Format                                  | Magic / version                                   |
| --------------------------------------- | ------------------------------------------------- |
| Key envelope (also the backup envelope) | Magic `IDKEY`.                                    |
| Invite                                  | Magic `IDINV`, version 2.                         |
| Presence frame                          | Magic `id-presence/v1` followed by `\n`.          |
| Peer announcement frame                 | Magic `id-peer-announcement/v2` followed by `\n`. |

## Remaining named domains and versions

| Format                           | Value                                                                                                                                                         |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Match-audit format               | Version `0`; domains `id.match-audit.v0/header`, `id.match-audit.v0/entry`, `id.match-audit.v0/footer`.                                                       |
| Boundary evidence domain         | `id.boundary.evidence.v1`.                                                                                                                                    |
| Boundary declaration hash domain | `id.boundary.declaration.v1`.                                                                                                                                 |
| Currency receipt domain          | `id.currency.receipt.v1`.                                                                                                                                     |
| Array address format (id-core)   | `FORMAT_VERSION 0x01`.                                                                                                                                        |
| Typed value format (id-core)     | `FORMAT_VERSION 0x02`.                                                                                                                                        |
| Config schema                    | `id-config` version `1`, plus the negotiation transcript domain `id-config/negotiation-transcript/v1`; see [Configuration and environment](configuration.md). |

## Test vectors

`pkgs/id/test-vectors/` contains exactly 3 files. They are conformance fixtures, not stores.

| File                              | Schema / content                                                                                                                                                                                                                                                                                                                                                                                 |
| --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `schema.json`                     | A JSON Schema (draft 2020-12) titled "ID/Atlas shared conformance test vectors", with `schema_version` const `1`. `port_kind` is one of `record_store`, `blob_store`, `event_log`, `world_store`, `object_bucket`, `message_stream`, `request_transport`. Operations are `negotiate`, `append`, `read`, `ack`. Negotiation outcomes are `granted`, `unsupported_capability`, `version_mismatch`. |
| `atlas-content-hash-v1.json`      | `"schema": "atlas-content-hash-compatibility-v1"`. It records the Atlas `chunks.content_hash` field as `producer_unavailable` with empty vectors, and states the interop claim verbatim: "forbidden: do not equate Atlas chunks.content_hash with any ID ContentId profile".                                                                                                                     |
| `legacy-artifact-signatures.json` | `"schema": "id-atlas-signed-artifact-profiles/v1"`. It pins the legacy `id-artifact-serde-json` signed-bytes definition, the SHA-256 hash rule and the Ed25519 signature rule, using a test-only Ed25519 key (seed = 32 × `0x07`). The file itself warns that this key is never a production key.                                                                                                |

## Worked example

Toy data: a node stores one file, renames it twice in the same second, and an operator reads the resulting on-disk state.

```text
> put report.txt report
> rename report report-v2
> rename report-v2 report-final
```

What exists afterwards:

| Place             | Entry                                                                                                                                                                                                                         |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `.iroh-store/`    | One blob (the toy bytes), referenced by the live name `report-final`; the redb name index holds the archive names below.                                                                                                      |
| Tag store         | `report-final` carries the identity keys `name`, `file`, `path` and the first-write `origin` key, plus `archive.replace` naming its archives.                                                                                 |
| Archive names     | `report.archive.1700000000` and `report-v2.archive.1700000000`, each carrying `archive.owner`. Because both renames landed in the same second, the second archive's suffix was bumped until free if the first claim collided. |
| Name refusal      | A later attempt to store a user file literally named `report.archive.1700000000` is refused with `ReservedArchiveName`; `report.archive.notes` would be accepted.                                                             |
| `events.redb`     | The `meta` table reads `id.events.store.v9`; the rename operations appear as events whose headers are canonical CBOR under the domain tag `id.events.v1`.                                                                     |
| `directory.jsonl` | Unchanged by file operations: it records account, group, member, tier and friend entries only — 17 kinds, none of them a file kind.                                                                                           |
