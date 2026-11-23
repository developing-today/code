# Design: `fleet kv` — namespaced key-value store (local + global), blob offload to content-addressed R2

- Status: draft (agent-preliminary; no human confirmation claimed)
- Date: 2026-11-23T12:45:00Z
- Roadmap item: none
- Issues: https://github.com/developing-today/code/issues/10359 (design owner), https://github.com/developing-today/code/issues/9107 (ID core epic: namespaces/ID shapes), https://github.com/developing-today/code/issues/9120 (content-addressed namespace core), https://github.com/developing-today/code/issues/9084 (content-addressed media/data plane), tracking https://github.com/developing-today/code/issues/12892
- Standalone design: there is no workflow research package for this item (no S0–S5 intake/lease state exists); this record is the decision record #10359 acceptance asks for, adapted honestly to that fact.

## Intent

Drew (2026-11-14, #10359) asked for a `fleet kv get/set/list` verb family in the `fleet` multicall binary for arbitrary bytes, explicitly not the secrets store. Each host has a local store (backed up); there is also a central store. Reads resolve local first and fall back to global; a local value overrides the same key in global. An explicit global operation (`fleet kv --global …`, exact surface TBD) always addresses central. Keys live in namespaces; a "global namespace" is a namespace designated global, not a second mechanism. Namespaces map to the ID project's namespace concept (#9107 / #9120), not a fleet-private reinvention. A session's local partition must survive disconnect: a sweeper pushes it into the central store keyed under the owning user id (+ context id). Every fleet-launched session — and other partitions (a repo, the fleet project itself, the mcpx namespace) — can be handed its own small database, likely Turso, with the embedded-replica attraction (fast local reads, automatic sync). The design must answer, with a workload sketch, what five agents concurrently pulling (and writing) a shared mid-size namespace actually get: replica sync semantics, writer topology, conflict behaviour — stated, not assumed. Values are arbitrary bytes; above a configurable threshold (Drew named 250 KB / 500 KB / 1 MB / 1 GB as the knob shape), a push keeps bytes local-working but routes them on central sync through a trusted Cloudflare endpoint into the content-addressed R2 bucket, storing key/namespace/name plus content hash + bucket link (#9084's reference pattern). Later, lists and tables ride the same substrate — design the KV layer so they are additive.

Today nothing of this exists: fresh greps find no `fleet kv` surface in the fleet crates (`pkgs/fleet/crates/`, command reference `pkgs/fleet/docs/verbs.md`) and no fleet-kv design under `doc/` (confirmed by the 2026-11-17 triage note on #10359). The fleet pieces that do exist and bound this design: the multicall/registry surface (`pkgs/fleet/docs/verbs.md`, `pkgs/fleet/crates/fleet-registry`), broker-token credential resolution by name (`pkgs/fleet/docs/fleet-gh-credentials.md`, `pkgs/fleet/crates/fleet-secrets/src/lib.rs:1-46`, `pkgs/fleet/crates/fleet/src/gh.rs:497-502`), the single front door for agents (`doc/2026-11-21T11-30-00Z_design_fleet-agent-front-door/2026-11-21T11-30-00Z_design_fleet-agent-front-door.md`), and Turso-backed queue state over the libSQL HTTP pipeline (`pkgs/fleet/crates/fleet-queue/src/turso_queue/core.rs:1-81,532-770`).

## Design

### D-anchored summary

Local stores are plain per-host SQLite files (WAL) in v1 (D001). The central store is one Turso-hosted libSQL database (the fleet-kv primary), addressed over the same HTTP pipeline pattern the queue already uses — not a per-session Turso DB and not libSQL embedded replicas in v1 (D002). Namespaces are a small registry table whose global flag is designation only, and whose content-blob identity reuses the settled #9120 profile byte-for-byte (D003, D010). Reads are local-first with local-override; `--global` pins central; deletes are tombstones with revert (D004, D005). Disconnect sweep is an explicit, idempotent, CAS-guarded merge keyed under `(owner_user_id, context_id, session_id)` (D006, D007). The five-agent shared-namespace answer is single-writer-per-key by CAS against the central primary, short-TTL reads, and no offline fork in v1 (D008). Blobs cross a per-namespace threshold (default 256 KiB, agent-preliminary) with a per-push override flag; large bytes go staged → verified → published through the trusted Cloudflare endpoint and are stored as references, never bearer hashes (D009–D012). Lists/tables are typed compositions over keys, additive (D013).

### 1. Store layout

```text
host local (SQLite file, WAL)              central (Turso libSQL primary)
+----------------------------+            +----------------------------------+
| kv_local                     |            | kv_namespaces                      |
|  (ns, key, value|ref, ver,   |  sweep   |  (ns, global_flag, threshold,     |
|   tombstone, origin, ts)     | ---------> |   policy_epoch, owner_scope)       |
| kv_namespaces (cache copy)   |  CAS merge | kv_central                        |
| outbox (sweep/upload queue)  |            |  (ns, key, value|ref, ver,         |
| backup manifest              |            |   tombstone, owner, ctx, ts)       |
+----------------------------+            | kv_blobs (references only)         |
        ^ file backup                   |  (content_id, bytes_len, locator,  |
        | (snapshot + WAL, §7)          |   partition, state)                |
                                        +----------------------------------+
                                                    | staged → verified → published
                                                    v object plane (R2/RustFS adapter)
```

Tables above are logical names for the phased build issues, not a claim that schema already exists. No `fleet kv` schema exists on main today.

**Local (D001).** One SQLite file per host, e.g. `~/.local/share/fleet/kv/local.db`, WAL mode, with `kv_local`, a cached copy of `kv_namespaces`, and a durable `outbox` for sweep/upload work. Rationale: the local store must accept writes while offline, survive process death, and back up as a file — plain SQLite does all three with zero network dependency. Two Turso products are distinguished and neither is the v1 path: (a) legacy embedded replicas — reads local, default writes remote to the cloud primary then reflected back, explicit `sync()` pulls, per-writing-replica read-your-writes (https://docs.turso.tech/features/embedded-replicas/introduction; docs now recommend new Turso Sync for new projects); (b) new Turso Sync — local-first writes with explicit `push()`/`pull()`, logical CDC, and last-push-wins on conflict (https://docs.turso.tech/sync/usage, https://docs.turso.tech/sync/conflict-resolution) — which collides with the CAS-per-key rule §5 requires. So: SQLite locally in v1 with a conservative outbox + central transaction sequencer; revisit embedded replicas or Turso Sync only as a named follow-up with a convergence proof, not as the v1 path. Do not conflate the two products/APIs or assume automatic multi-master CAS.

**Central (D002).** One fleet-kv Turso database as the libSQL primary for KV rows and blob *references*; bytes live in the object plane (§6). Clients that need central access use the same transport shape the queue already proves: libSQL HTTP pipeline from Rust (`pkgs/fleet/crates/fleet-queue/src/turso_queue/core.rs:81,671-770`, `DEFAULT_TURSO_URL` at `core.rs:10` shows the pattern, not the kv database). Central writes serialize per key on the primary's transactions (conservative outbox + central transaction sequencer, §5); this design assumes no automatic multi-master CAS from any sync product. Per-user Turso DB credentials (component-E data-plane tokens) are not service authentication and are refused at the hosted front door by rule (`pkgs/fleet/hosted-worker/src/auth.ts:20-33,134-159` `data_plane_credential_refused`); central KV access goes through the broker/pipeline path, never by presenting a data-plane token. Per-group "small databases" from the issue (one per session/repo/project/mcpx namespace) are a provisioning option *behind* this primary — either separate Turso DBs reached over the same pipeline, or row-partitioned scopes inside the one primary — but they are not separate sync topologies in v1. The v1 topology is exactly: many SQLite locals → one central primary → one object plane.

**Backup/restore (§7, D001 detail).** The local file is backed up as a file: periodic SQLite backup API or filesystem snapshot of file + WAL, plus a `backup manifest` row (host, file hash, WAL frame, timestamp) kept in the outbox so restore can prove which snapshot it holds. Restore is: fetch latest snapshot, open read-only to verify, then either adopt (fresh host) or three-way merge through the §5 sweep path (returning host with divergent local rows). Backup never writes bytes into the KV row space; large local working bytes above threshold stay local-working until §6 publishes them.

### 2. Namespaces (coordinate with #9107 / #9116, not parallel)

Namespaces reuse the ID local-namespace-registry concept: a namespace definition binds `{namespace_id, core_id, handler/encoding profile, read/write policy, shorthand policy, schema version}`, server-local, versioned and audited (`doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md`, §#9116). Fleet-kv does not mint a second namespace grammar:

- `kv_namespaces` rows carry `(namespace_id, core_binding, global_flag, blob_threshold, policy_epoch, owner_scope)`.
- `global_flag` is designation only (D003): "global namespace" = a namespace whose reads/writes default to central. No second registry, no second lookup path.
- Content-blob identity inside any namespace is exactly the settled #9120 initial profile: unkeyed BLAKE3-256 over exact bytes, 32-byte digest (A-9120.1; research `part-a-addressing/research.md` #9120 row; implementation slices `part-a-addressing/implementation-slices.json` A9120-content-id). Fleet-kv neither redefines the hash nor invents a parallel digest (binding constraint shared with #9120's "neither this spec nor §3's may redefine the hash").
- Parse/URI transport, if ever exposed as `id://`, follows #9117 syntax transport only; this design mints no IDs and establishes no authority beyond the namespace's own write policy.

Coordination boundary: #9116 owns the registry model; #9117 owns URI syntax; #9118 owns the array core; #9120 owns the immutable content core. Fleet-kv is a consumer of all four and defines only the KV row envelope, the override/sweep rules, and the blob reference row below. It preserves Part A untouched: no redefinition of the hash, no new core, no parallel namespace grammar. Not this surface: the F12.5 in-world `kv`/`blob` providers (#1062: `kv.get/set/delete` with 8 KiB values / 1024 entries / 64 KiB per world, `doc/2026-11-22_design_id1062-kv-provider.md`) are world-sandbox ceilings, not the fleet CLI store — this design neither implements nor changes them.

### 3. Read/list/set/delete/tombstone/revert semantics

Verbs: `fleet kv get|set|list` plus `delete` and `revert` (delete without revert is a half-design; the sweep needs tombstones to converge). `--global` pins central for the verb it modifies; `--namespace NS` selects scope; `--as-bytes-ok` style escape hatches are out of scope for v1.

- **Read path:** local first; on miss, fall back to global/central. A live local row *overrides* the same key in global (D004). `--global get` skips local entirely. `list` merges both scopes with origin marks (`local|central|both`, local winning display) and never silently hides the shadowed central row — show it as `shadowed-by-local` in verbose output.
- **Set path:** default writes local + enqueues outbox work; `--global set` writes central directly (authenticated; §6 for large values). Local sets carry `(key, namespace, value-or-ref, base_version, writer_session, ts)`.
- **Delete:** writes a tombstone row `(key, namespace, tombstone_epoch, deleter, ts)`, never a physical row removal (D005). Reads treat tombstoned keys as absent but `list --include-tombstones` and `revert` can see them. Tombstones sweep exactly like values (§5), which is what makes two-session delete/write converge instead of resurrecting.
- **Revert:** `fleet kv revert NS KEY --to <version>` writes a new version whose value equals the named prior version (new version number, full provenance) — history is append-only; there is no in-place rollback. Reverting a tombstone re-lives the key at a new version.
- **Freshness:** local reads are immediate; central reads are bounded-stale only by explicit `sync`/`pull` in v1 (no background sync promise). Whatever freshness knob ships must be a documented flag, not ambient behaviour.

### 4. Durable session-partition sweep

A session's local partition does not evaporate at disconnect (D006). On session end (and on explicit `fleet kv sync`), a sweeper merges the session's dirty rows into central under `(owner_user_id, context_id, session_id)` provenance:

1. Read session dirty set from `outbox` (durable, so a dead session still sweeps on next host run).
2. For each key: `central_CAS(key, expected=base_version, new=row)` against `kv_central`.
3. On success: mark outbox row done. On conflict: run the §5 rule (versioned conflict, never silent last-writer-wins across sessions), record the conflict row, and leave both versions retrievable.
4. Tombstones sweep through the same CAS path.
5. Large values sweep by reference: the sweeper uploads through §6 first, then CASes the reference row. Central never accepts an unpublished blob reference.

Sweep is idempotent (outbox row ids + CAS make re-runs safe) and recoverable (crash between upload and CAS leaves a staged blob that staging TTL reaps, per #9084 component-F staging rules; crash between CAS and outbox-ack re-runs CAS harmlessly). "Two sessions sweeping the same key" is therefore a routine CAS conflict, not a special case.

### 5. CAS/versioned conflicts, retry, idempotence, recovery

Every mutable head uses compare-and-swap on `(key, namespace, version)` (D007), following the #9114 sidecar-head precedent (CAS on prior `(generation, snapshot_id)`; generation increments exactly once). There is no timestamp-only last-writer-wins anywhere in v1:

- Each row carries `version` (monotonic per key) + `parent_version` + writer identity. A writer that did not read the current version fails CAS with `conflict {key, expected, actual}` and retries by re-reading, re-applying intent, and re-CASing — or surfaces the conflict to the user for genuinely incompatible intents.
- Retries are bounded with jitter and carry idempotency keys (outbox row id = the idempotency key; re-sending the same CAS is a no-op once applied).
- Recovery order is fixed: outbox → upload (§6) → CAS → ack. Any crash restarts at the oldest unacked outbox row. Staged-but-unpublished blobs are never referenced by any read path, so they are safe to reap by TTL.
- `concurrency_unsupported` is a typed error if some future adapter cannot CAS: fail closed, never degrade to silent overwrite (same posture as the V2 store seam in `part-a-addressing/versioned-design.md`).

### 6. Five agents on one shared mid-size namespace (workload sketch + topology)

Workload assumed (agent-preliminary planning fixture, stated so a later measurement can replace it — not a measured or free-tier guarantee): namespace holds ~10k–100k small keys (hundreds of bytes to a few KiB each; larger values are references per §8), five agents each holding the namespace open for minutes, read-heavy with intermittent writes (each agent writes ~1–10 keys/min, occasionally the same key family, e.g. status/claim rows). A "five agents × ~100 MiB" shape, where named, is the same kind of planning fixture, not a throughput promise.

- **Writer topology (D008): single-writer-per-key by CAS against the central primary.** There is no multi-master convergence in v1: all writers serialize per key on `kv_central`'s CAS. The "shared mid-size namespace" is therefore N readers of a locally cached copy plus M writers arbitrated centrally per key — the same shape as the fleet queue's claim path (atomic claim over Turso in `pkgs/fleet/crates/fleet-queue/src/turso_queue/claim.rs`), not a mesh sync.
- **Replica sync semantics:** no embedded replicas in v1 (D002 rationale). Agents read central directly (short-TTL client cache, TTL documented per namespace) or read a snapshot pulled at task start; there is no background replica convergence to reason about, and therefore no divergence window beyond the stated cache TTL. Offline writes in v1 are local-only and unsynced — they become visible only through the §4 sweep, and conflicts there follow §5.
- **Freshness:** reads state their source (`local|central@version|cache@ttl-expiry`); claim/status keys that need linearizability use `--global` reads + CAS writes. Nothing else gets linearizability, and the CLI says so.
- **Contention:** same-key write contention fails CAS fast; hot keys should be claim/checkpoint-shaped (queue pattern) rather than counter-shaped. No counters in v1 KV rows — a stored counter is a second copy of the truth and drifts (same argument as `doc/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane.md` §6 against reference counting).
- **Cost:** central rows are small (values inline only at/below threshold; §8). Five agents polling `list` at 1 Hz against Turso HTTP is the abuse case: mitigate with the documented cache TTL, snapshot reads, and (if needed) a per-namespace read budget owned by the same policy epoch as the threshold. No per-request billing claim is made here — measure on the fleet-kv Turso DB once Phase 2 lands.

Why not embedded replicas for the five-agent case: embedded-replica writes go to the cloud primary anyway (https://docs.turso.tech/features/embedded-replicas/introduction), so five replica-holders writing the same key still serialize centrally — replicas add a divergence/catch-up window without removing the central serialization point. v1 keeps that serialization explicit and auditable.

### 7. Host-local technology choice: SQLite vs embedded libSQL/Turso

D001/D002 decide: **plain SQLite (WAL) per host for local; Turso-hosted libSQL primary for central; no embedded-replica mode in v1.** Backup story: local = file snapshot + WAL + backup-manifest row (§1); central = Turso's own durability plus the tombstoned, versioned row history (point-in-time story is the provider's, not a second log we maintain). Sandbox hosts ("sandbox has their own local store that maybe we back up and merge") get exactly the §4 sweep: their partition merges on session end, and a dead sandbox's outbox merges on next host run — "merge" always means CAS merge, never file copy.

### 8. Arbitrary bytes, streaming, blob threshold knob

Values are arbitrary bytes with a streaming interface (no full-buffer requirement on push/pull above a small in-memory cap). The threshold knob (D009):

- **Per-namespace policy value** (`kv_namespaces.blob_threshold`, bytes), default **256 KiB (agent-preliminary)** — inside Drew's named shape (250 KB / 500 KB / 1 MB / 1 GB) at its low end. This default is subordinate to the shared per-partition placement policy, not a competing protocol: where KV bytes and #9084 component-F content share a partition, component E owns the threshold and the 16 KiB conservative starting default (`doc/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane/...md` §Recommendation) governs placement until #9098 measurements move it. The KV per-namespace knob may only narrow (force-blob sooner), never widen past the partition policy, and changing either changes placement of new content only, never identity or access (policy, never protocol).
- **Per-push override flag** (`--inline-max BYTES` / `--blob-threshold BYTES`, exact flag name owned by Phase 1): both directions supported — force-inline a just-over-threshold value or force-blob a small value that should not live in the row store.
- At or below the effective threshold: bytes inline in the row. Above: §6 reference path. Promotion (inline → referenced) is a new version with identical read semantics; no client-visible change.

### 9. Trusted Cloudflare endpoint, broker-token auth, reference-based read (no bearer hash)

Auth reuses the fleet's existing shape, not a new credential (D011):

- **Push path:** the pusher authenticates to the trusted Cloudflare endpoint with its **existing fleet broker token** (same family as the secrets-broker token file pattern: `fleet-secrets` resolves by name, broker-first; `pkgs/fleet/docs/fleet-gh-credentials.md`, `pkgs/fleet/crates/fleet-secrets/src/lib.rs:15-28`). The endpoint verifies the token, checks the namespace write policy for `(user, context, namespace)`, and only then accepts staged bytes. The content hash travels as an *assertion to be verified*, never as authority.
- **Read path:** a bearer hash grants nothing (binding rule shared with `doc/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane.md` §5 "the hash is not a capability"). Readers resolve their KV row authorization first (local or central, per namespace policy + authenticated context), then receive a **short-lived, object+operation-scoped capability** minted from live reference state; the adapter serves bytes only under that capability, with replica fallback during placement movement (same contract as component I's object adapter: `doc/2026-11-21T21-40-00Z_design_self_hosted-tier-adapters/2026-11-21T21-40-00Z_design_self_hosted-tier-adapters.md`, object adapter contract).
- **Upload safety:** `put_staged → verify (length + BLAKE3-256 recomputed on receipt, streaming) → publish`. Partial uploads are staging state, never accepted objects (same invariant as A-9120.1 and component-F staging). Multipart completion is not publication; only the service verification step makes the reference row writable.
- **GC safety:** mark-and-sweep per partition exactly as component F §6 (references are the rows; derived index is a cache; re-verify at grace expiry; epoch-fenced delete; audit event per delete). Fleet-kv blob references are rows in `kv_central`/`kv_blobs`, so they slot into that protocol as one more reference source; per-jurisdiction dedup partitions from #3401 bound every sweep (D012).

### 10. Per-jurisdiction dedup and reference format (using existing ID hash shapes)

- Dedup partitions are `(jurisdiction, sensitivity tier)` per #3401, applied inside the object plane per component F §4. Cross-partition "dedup" is prohibited, not a missed saving (existence leak). Fleet-kv rows carry the partition on the blob reference; a reference resolving across partitions refuses (typed `partition_mismatch`), mirroring component F's cross-partition refusal.
- Reference format (D010) reuses existing shapes only:

```text
KvBlobRefV1 {
  profile: "raw-blake3-256",          // the #9120 initial profile token, no new token
  digest: 64lower-hexdig,             // 32-byte BLAKE3-256 over exact bytes
  bytes_len: uint64,
  partition: { jurisdiction, tier },   // #3401 / component-F §4
  locator: opaque adapter locator,    // R2/RustFS key private to the adapter; never a stable API
  state: staged | verified | published
}
```

No new hash, no new multicodec, no bearer URL persisted as identity. The locator is adapter-private (component-I peer principle: no client-visible branch on backend kind).

### 11. Additive lists and tables (D013)

Lists and tables are typed compositions over keys, never a second storage engine:

- **List:** a key whose value is a signed manifest of member references (ordered member key names or blob refs + generation). Append = CAS on the manifest head (same rule as #9127 append-list user attributes: `doc/2026-11-17_design_id9127_append_list_user_attributes.md`, and #9118 array-address ordering where order matters).
- **Table:** a namespace-scoped prefix convention (`<table>/<row>/<col>` keys) plus a schema manifest key (column types per #9122 numeric kinds where numerics are keys: `Integer|Decimal|BinaryFloat`, no implicit coercion). Reads compose from point gets; no SQL surface (explicit non-goal of #10359).
- Both inherit namespaces, CAS, tombstones, sweep, thresholds, and GC unchanged — they add client-side composition and one manifest schema, nothing else.

## Decisions

All `agent-preliminary`: work may proceed on these defaults; only a human moves one to confirmed or reversed (see `human-review.md` for the genuine toss-ups).

- **D001 — Local store is plain per-host SQLite (WAL), not embedded libSQL/Turso, in v1.** Source: issue #10359 item 2 + backup question; evidence: offline-write and file-backup needs (§1, §7); Turso embedded-replica write path (writes to cloud primary; https://docs.turso.tech/features/embedded-replicas/introduction). Status: agent-preliminary. Reversal cost: medium (replaces local engine + backup story; central protocol unchanged). Human review: needed only if v1 must survive host disk loss without sweep (then embedded replicas don't help either — say so).
- **D002 — Central store is one Turso-hosted libSQL primary; per-group DBs are provisioning scopes, not sync topologies, in v1.** Source: #10359 items 2/5; evidence: queue's Turso HTTP pipeline precedent (`pkgs/fleet/crates/fleet-queue/src/turso_queue/core.rs:81,671-770`); embedded-replica central-serialization argument (§6, Turso docs). Status: agent-preliminary. Reversal: high (splits the primary). Human review: needed (cost + tenancy shape).
- **D003 — Global namespace = designation flag on the shared registry, no second mechanism.** Source: #10359 item 3; aligns #9116 registry (versioned-design §#9116). Status: agent-preliminary. Reversal: low. Human review: not needed (directly what the issue asks).
- **D004 — Local-first reads with local-override; `--global` pins central; merged list shows shadowed rows.** Source: #10359 item 2 read path. Status: agent-preliminary. Reversal: low. Human review: not needed.
- **D005 — Deletes are tombstones; `revert` appends a restoring version.** Source: sweep-merge need (#10359 item 4) + #9115 tombstone reconciliation precedent. Status: agent-preliminary. Reversal: medium (resurrection bugs if dropped). Human review: not needed unless true-delete has a compliance deadline (then see #9130 true-delete coordination).
- **D006 — Sweep merges under `(owner_user_id, context_id, session_id)` via durable outbox + CAS.** Source: #10359 item 4 + context-propagation note filed the same day. Status: agent-preliminary. Reversal: medium. Human review: needed on keying (user+context vs session-only expiry).
- **D007 — All mutable heads are CAS-versioned; no timestamp LWW; bounded-jitter retry with outbox idempotency keys.** Source: #9114 head rule (CAS on prior generation); store-seam `concurrency_unsupported` posture. Status: agent-preliminary. Reversal: high (correctness invariant). Human review: not needed (invariant, not a toss-up).
- **D008 — Five-agent shared namespace: single-writer-per-key CAS on central; short-TTL reads; no offline fork in v1.** Source: #10359 item 5 question; evidence: claim-path precedent (`fleet-queue/.../claim.rs`); legacy embedded-replica write path (writes to cloud primary; https://docs.turso.tech/features/embedded-replicas/introduction) and Turso Sync last-push-wins (https://docs.turso.tech/sync/usage, https://docs.turso.tech/sync/conflict-resolution) both noted and rejected as the v1 conflict rule. Status: agent-preliminary. Reversal: medium. Human review: needed (freshness/cost tolerance is a product call).
- **D009 — Blob threshold is per-namespace policy (default 256 KiB agent-preliminary) with a per-push override flag, subordinate to the shared partition policy.** Source: #10359 threshold question; precedent: component-F "threshold is policy, never protocol" with 16 KiB conservative start. Status: agent-preliminary. Reversal: low (policy value). Human review: needed (default + flag shape).
- **D010 — Blob identity is exactly #9120 `raw-blake3-256` over exact bytes; reference row format `KvBlobRefV1` reuses that profile.** Source: #9120 A-9120.1 + #9113 shape dispatch; research `part-a-addressing/research.md` #9120 row. Status: agent-preliminary. Reversal: high (identity change). Human review: not needed (follows settled profile).
- **D011 — Broker-token auth to the trusted Cloudflare endpoint; hash is never a bearer credential; reads mint short-lived scoped capabilities.** Source: fleet credential shape (`fleet-gh-credentials.md`, `fleet-secrets`); component-F §5 access rule. Status: agent-preliminary. Reversal: high (auth architecture). Human review: needed (endpoint trust + capability TTL are product/security calls).
- **D012 — Blob GC is component-F mark-and-sweep per `(jurisdiction, tier)` partition; kv references are rows the sweep reads.** Source: component-F §6 + #3401 partitions + #9137 priority note. Status: agent-preliminary. Reversal: medium. Human review: not needed unless grace defaults change.
- **D013 — Lists/tables are manifest compositions over keys; no SQL surface.** Source: #10359 item 6 expansion note; precedents #9118/#9122/#9127. Status: agent-preliminary. Reversal: low. Human review: not needed.

## Alternatives considered

- **Embedded libSQL replica as the local store.** Lost: writes still serialize on the cloud primary (Turso docs), so it buys a divergence window without removing central arbitration, and it needs network for the write path that session partitions need most. Revisit only with a convergence proof and an offline story.
- **Turso Sync (`push`/`pull`) as the sync substrate.** Lost for v1: local-first and explicit, but conflict strategy is last-push-wins (Turso docs, https://docs.turso.tech/sync/usage), which contradicts the CAS-per-key invariant §5 needs for sweep/claim correctness. Possible later as a transport under CAS, not as the conflict rule.
- **Per-session (or per-group) Turso DB as the unit of sync.** Deferred, not rejected: matches Drew's "handed its own small database" intuition and may become the provisioning story, but in v1 it multiplies primaries without settling the per-key arbitration question. D002 keeps one primary; D006 keeps group scoping at the row/namespace level.
- **Reference-only-everything (no inline threshold).** Lost: for small KV values the reference machinery (staging, capability mint, GC sweep) costs more than the bytes it saves — the same argument component F makes for inline small content, applied one tier down in size.
- **Copy-bytes-into-central-DB (no object plane).** Lost: prices media as database, abandons dedup, and makes publication a divergent copy — component F's evaluation already rejects this for content above threshold, and KV values above threshold are the same case.
- **Bearer-hash reads (hash grants bytes).** Rejected outright: hashes are enumerable and leak by design (sidecars, location index #9115); serving on hash presentation alone fails the access-control soundness axis of #9084. Never the default; not even a flag.
- **Timestamp last-writer-wins for sweep conflicts.** Rejected: loses concurrent sessions' work silently and reintroduces ABA the #9114 head rule already forbids. CAS conflict with both versions retrievable is the floor.

## Consequences

- Gets easier: session work product survives disconnect by construction; namespaces stay one system with ID (#9107 family) instead of a fleet-private dialect; large values stop pricing as database rows; lists/tables arrive without a second engine; every conflict is a retrievable record, not silent loss.
- Gets harder: every writer implements CAS retry; every large push does staged→verified→published instead of a single write; reads that need linearizability must say `--global` explicitly; GC/tombstone literacy becomes operator knowledge.
- Must stay compatible: #9120 hash profile bytes (never rehash/reinterpret); legacy local state (adopt-or-CAS-merge, never overwrite); broker-token auth shape (no new credential); adapter neutrality (R2 vs RustFS invisible above the adapter, per component I).

## Open questions

- **O1 — `--global` surface exactness:** flag on every verb vs `fleet kv global <verb>` subcommand. Default in effect: flag (`--global`), matching the issue's sketch. Settles: Phase 1 build (human-review H3).
- **O2 — Sweep keying:** `(owner_user_id, context_id)` with session provenance vs session-expiry alternative. Default: owner+context keying (D006). Settles: Drew/product (human-review H1).
- **O3 — Threshold default + flag name.** Default: 256 KiB per namespace + per-push override (D009). Settles: measurement (#9098-style KV workload) + Phase 1 UX (human-review H2).
- **O4 — Capability TTL + revocation expectations.** Default: short-lived (minutes), revocation = reference removal + TTL-bounded lag (component-F posture). Settles: security/product (human-review H4).
- **O5 — Central tenancy billing/placement:** one primary vs per-group DBs at scale. Default: one primary in v1 (D002); deferred to measurement. Settles: Phase 2 load evidence.
- **O6 — List/table manifest schema versioning.** Deferred to plan: Phase 3 details manifest v1 + upgrade rule; no default needed before then.
- **O7 — True-delete/retention interplay** with #9130-versioning/#3401 erasure flows for tombstoned KV rows. Deferred to plan; D005 tombstones are compatible with either outcome.
