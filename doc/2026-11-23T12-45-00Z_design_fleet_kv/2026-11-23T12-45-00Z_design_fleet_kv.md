# Design: `fleet kv` — namespaced key-value store (local + global), blob offload to content-addressed R2

- Source: agent-preliminary (all defaults below are the writer's defaults; no human confirmed them)
- Status: preliminary
- Date: 2026-11-23T12:45:00Z (reconciled 2026-11-23 after read-only review; challenge outcomes 2026-11-23 integrated)
- Roadmap item: none
- Issues: https://github.com/developing-today/code/issues/10359 (design owner), https://github.com/developing-today/code/issues/9107 (ID core epic: namespaces/ID shapes), https://github.com/developing-today/code/issues/9120 (content-addressed namespace core), https://github.com/developing-today/code/issues/9084 (content-addressed media/data plane), tracking https://github.com/developing-today/code/issues/12892
- Design PR: https://github.com/developing-today/code/pull/13294 — must merge before any phase build starts.
- Standalone design: there is no workflow research package for this item (no S0–S5 intake/lease state exists); this record is the decision record #10359 acceptance asks for, adapted honestly to that fact.
- Pre-change snapshot (not "no design exists"): as of the 2026-11-14 triage note on #10359, greps found no `fleet kv` surface in the fleet crates and no fleet-kv schema on main. This document, created 2026-11-23, is the first such record; "nothing exists" below means no implementation, not the absence of this doc.

## Intent

Drew (2026-11-14, #10359) asked for a `fleet kv get/set/list` verb family in the `fleet` multicall binary for arbitrary bytes, explicitly not the secrets store. Each host has a local store (backed up); there is also a central store. Reads resolve local first and fall back to global; a local value overrides the same key in global. An explicit global operation (`fleet kv --global …`, exact surface TBD) always addresses central. Keys live in namespaces; a "global namespace" is a namespace designated global, not a second mechanism. Namespaces map to the ID project's namespace concept (#9107 / #9120), not a fleet-private reinvention. A session's local partition must survive disconnect: a sweeper pushes it into the central store keyed under the owning user id (+ context id). Every fleet-launched session — and other partitions (a repo, the fleet project itself, the mcpx namespace) — can be handed its own small database, likely Turso, with the embedded-replica attraction (fast local reads, automatic sync). The design must answer, with a workload sketch, what five agents concurrently pulling (and writing) a shared mid-size namespace actually get: replica sync semantics, writer topology, conflict behaviour — stated, not assumed. Values are arbitrary bytes; above a configurable threshold (Drew named 250 KB / 500 KB / 1 MB / 1 GB as the knob shape), a push keeps bytes local-working but routes them on central sync through a trusted Cloudflare endpoint into the content-addressed R2 bucket, storing key/namespace/name plus content hash + bucket link (#9084's reference pattern). Later, lists and tables ride the same substrate — this design reserves only a versionable bytes/reference envelope for them (D015); list/table semantics are deferred, not designed here.

Today no implementation exists (pre-change snapshot above). The fleet pieces that do exist and bound this design: the multicall/registry surface ([verbs.md](../../pkgs/fleet/docs/verbs.md), `pkgs/fleet/crates/fleet-registry`), named-secret resolution via the secrets broker ([fleet-secrets lib.rs](../../pkgs/fleet/crates/fleet-secrets/src/lib.rs), [fleet-gh-credentials.md](../../pkgs/fleet/docs/fleet-gh-credentials.md) — a secret-*fetch* protocol, not request authentication), the single front door for agents ([fleet-agent-front-door design](../../doc/2026-11-21T11-30-00Z_design_fleet-agent-front-door/2026-11-21T11-30-00Z_design_fleet-agent-front-door.md)), hosted request authentication ([auth.ts](../../pkgs/fleet/hosted-worker/src/auth.ts): RP/agent JWTs or route-scoped service tokens; data-plane credentials refused), and Turso-backed queue state over the libSQL HTTP pipeline ([core.rs](../../pkgs/fleet/crates/fleet-queue/src/turso_queue/core.rs)).

## Design

### D-anchored summary

Local stores are per-host SQLite files (WAL) in v1 (D001; standalone Turso local-only steelmanned and rejected, §7). The central plane is hybrid (D014, supersedes D002): one shared Turso primary for shared namespaces; a real group database exists only via an explicit fail-closed grant `(group, database, allowed namespaces/ops)` under an admission/provisioning policy — no default per-session proliferation, no unproved pricing. Namespace resolution always names an explicit registry: per-host server registries plus one distributed fleet registry whose identity and replication path are defined in §2; `global_flag` is designation only and never bypasses the local-first read rule (D003, D004). Local rows carry the complete logical identity `(owner_user_id, context_id, partition_id, namespace, key)` so no local read can cross users/sessions (§1, §3). Local deletion distinguishes three states: clear-override (drop the overlay, reveal central), masking tombstone (locally absent, sweeps as a mask), and history revert (append a restoring version) (D005, §3). Disconnect sweep is an explicit, idempotent, CAS-guarded merge with append-only version/conflict history and atomic idempotency receipts (D006, D007, §4–§5). The five-agent shared-namespace answer is single-writer-per-key by CAS against the central primary, ordinary cached reads at most 1 s old, `--global` as the authoritative bypass, and offline intents that are local-only until sweep CAS commits them; KV CAS is not a queue lease (D008, §6). Blobs cross an effective threshold that is a hard inline ceiling: the KV per-namespace knob (default 256 KiB) may only narrow, never widen past the shared partition ceiling; per-push override forces blob earlier only (D009, §8). Uploads go staged → verified → published through the trusted endpoint authenticated by RP/agent JWT or route-scoped service token — never the secrets-broker fetch path — and reads mint short-lived capabilities from live reference state, never bearer hashes (D011, §9). Blob GC is component-F mark-and-sweep per `(jurisdiction, tier)` with the KV published-row set as the authoritative reference state sharing the per-partition serialization point (D012, §9–§10). Lists/tables are deferred: Phase 3 is blob-only; this design reserves only a versionable envelope (D015, §11).

### 1. Store layout and logical identity

```text
host local (SQLite file, WAL)                  central (shared Turso primary)
+----------------------------+            +----------------------------------+
| kv_local                       |            | kv_namespaces                      |
|  (owner, ctx, part, ns, key,   |  sweep   |  (ns, global_flag, threshold,     |
|   value|ref, ver, state, ts)   | ---------> |   policy_epoch, owner_scope)       |
| kv_namespaces (cache copy)     |  CAS merge | kv_heads (per-key head)            |
| outbox (sweep/upload queue)    |            | kv_versions (append-only history)  |
| kv_conflicts (local copy)      |            | kv_conflicts (append-only)         |
| idem_receipts (local copy)     |            | idem_receipts (outbox_id → result) |
| kv_grants (cache copy)         |            | kv_grants (group,db,ns,ops,epoch)  |
| backup manifest                |            | kv_blobs (published refs only)     |
+----------------------------+            +----------------------------------+
        ^ file backup                     |  (content_id, bytes_len, locator,  |
        | (snapshot + WAL, §7)            |   partition, state=published)      |
                                         +----------------------------------+
                                                     | staged → verified → published
                                                     v object plane (R2/RustFS adapter)
```

Tables above are logical names for the phased build issues, not a claim that schema already exists. No `fleet kv` schema exists on main today (pre-change snapshot).

**Complete logical identity.** Every local row is keyed by the full tuple `(owner_user_id, context_id, partition_id, namespace, key)`, where `partition_id` is the session id or the owning group (repo / fleet project / mcpx namespace). Central shared heads are keyed `(namespace, key)` with an `owner_scope` column; the survival archive (§7) retains the full tuple plus the sweep receipt. Consequences, stated as rules:

- A local read filters by the caller's full identity: `(owner, ctx, partition)` from the authenticated context. A local read can never return another user/session/partition's row — there is no `(ns, key)`-only lookup path anywhere in v1. The `(ns, key)`-only shape from the earlier draft is withdrawn.
- Central shared heads are a different mapping: `(namespace, key) → head` with `owner_scope` authorizing which identities may read/write it. Row-level scope is authorization metadata on a shared head, not a grant: unless the build ships an explicit scoped-facade (per-scope views enforced at the pipeline layer), all central access goes through the single primary's CAS path with the namespace write policy checked per write. No claim is made that "rows equal database grants."
- The survival archive is the third mapping: full-identity rows plus sweep receipts, written only by the sweep path, read only for restore/audit.

**Local (D001).** One SQLite file per host, e.g. `~/.local/share/fleet/kv/local.db`, WAL mode, holding `kv_local`, a cached copy of `kv_namespaces` and `kv_grants`, the durable `outbox`, and local copies of conflict/receipt rows for offline inspection. Rationale: the local store must accept writes while offline, survive process death, and back up as a file — plain SQLite does all three with zero network dependency. §7 records the investigated alternatives.

**Central (D014; D002 reversed/superseded).** Hybrid placement:

- **Shared namespaces (the v1 default):** one fleet-kv Turso database as the libSQL primary for KV heads, append-only version/conflict history, idempotency receipts, grant rows, and blob *references*; bytes live in the object plane (§6). Clients use the same transport shape the queue already proves: libSQL HTTP pipeline from Rust ([core.rs](../../pkgs/fleet/crates/fleet-queue/src/turso_queue/core.rs) lines 1–81 and 532–770; `DEFAULT_TURSO_URL` at line 10 shows the pattern, not the kv database). Central writes serialize per key on the primary's transactions (conservative outbox + central transaction sequencer, §5); this design assumes no automatic multi-master CAS from any sync product.
- **Group databases (explicit grant only):** a real per-group Turso DB exists iff a `kv_grants` row `(group, database, allowed namespaces, allowed ops, policy_epoch)` authorizes it, issued through a fail-closed admission/provisioning policy (admission checks group membership + namespace ownership; provisioning mints the DB, writes the grant row, and replicates the namespace subset; unknown group or namespace refuses). No default per-session proliferation: sessions do not get databases unless a grant says so. Not checked: per-database pricing and Turso org limits for grant-provisioned DBs — no cost claim is made; Phase 2 measures against the shared primary only.
- Per-user Turso DB credentials (component-E data-plane tokens) are not service authentication and are refused at the hosted front door by rule ([auth.ts](../../pkgs/fleet/hosted-worker/src/auth.ts) lines 20–33 and 134–159 `data_plane_credential_refused`); central KV access goes through the authenticated pipeline path, never by presenting a data-plane token.
- **v1 operational integration (required, not pretended-shipped):** the Phase 2 build must land migrations for all central tables above, the pipeline credential wiring, the grant/provisioning policy implementation, per-partition backup/restore runbook entries, and central health/readiness probes — a primary that none of this exists for is not a primary. Phase 2 acceptance (§4 of the phase-2 draft) gates on these, not on the row protocol alone.

**ID journal relationship (binding).** The ID serverless storage contract ([storage contract](../../doc/2026-11-17_decision_id5863_storage_contract.md)) fixes the signed journal as the system of record for ID world state; fleet-kv does not mint a competing journal. Scope boundary, stated exactly: KV row state is fleet-owned application state, not ID world state. Its durability contract is a bounded Fleet projection protocol — per-host outbox journal of *unsigned, fleet-scoped write intents* → central primary CAS transactions — which claims no ID-world authority, carries no ID signatures, and is never presented as an ID journal. Content blobs remain fully under the ID source-of-truth contract (content identity per #9120, admission per the signed gate); fleet-kv rows reference that content, they do not re-journal it.

**Backup/restore (§7, D001 detail; numbers are defaults in effect).** Two destinations, both concrete:

1. **Local file snapshot** (survives process death, not disk loss): SQLite backup API or filesystem snapshot of file + WAL into `~/.local/share/fleet/kv/backups/` every **6 h**, plus a `backup manifest` row (host, file hash, WAL frame, timestamp) in the outbox. RPO for process-death recovery: last write (WAL replay).
2. **Survival archive** (survives disk loss): the sweeper uploads the session dirty set — including large *working* bytes staged via §9, not inline — to the central archive scope on session end and every **15 min** during a live session (RPO ≤ 15 min for acknowledged local writes). Large working blobs are part of the snapshot: the manifest lists staged content ids, and restore re-publishes or re-links them.

Restore: fetch latest snapshot (local dir first, archive scope if the disk is gone), open read-only to verify, then adopt (fresh host) or three-way CAS-merge through §4 (returning host with divergent rows). **Durability boundary, stated plainly:** an acknowledged local write lost to disk destruction *before* the next 15-minute archive upload is unrecoverable — that is the loss window (≤ 15 min of local writes, plus any over-threshold working bytes not yet staged). Backup never writes bytes into the KV row space; large local working bytes stay local-working until staged.

### 2. Namespaces and registry identity (coordinate with #9107 / #9116, not parallel)

Namespaces reuse the ID local-namespace-registry concept: a namespace definition binds `{namespace_id, core_id, handler/encoding profile, read/write policy, shorthand policy, schema version}`, server-local, versioned and audited (Part A versioned design, §#9116). The ID contract constrains this design ([namespace_definition.rs](../../pkgs/id/crates/id-core/src/namespace_definition.rs) lines 24–32 and 594–645): registries are local, resolution always names a registry (`Resolver::resolve(registry, namespace)`), and there is no global registry or fallback. Fleet-kv therefore defines its registry identity explicitly instead of a "shared registry":

- **Fleet registry identity:** the central `kv_namespaces` table is the distribution source under registry handle `fleet-kv-central` (a `RegistryId::Handle`, [namespace_definition.rs](../../pkgs/id/crates/id-core/src/namespace_definition.rs) lines 424–440). Each host holds a server registry (`RegistryId::Server(host_id)`) whose fleet-namespace definitions are **replicas provisioned from** `fleet-kv-central`: on `fleet kv sync` (and at most every **1 h**, default in effect) the host pulls the central table at its watermark and replaces its cached definitionsassigning the central `policy_epoch`. Reads resolve in the host's server registry only; there is never a cross-registry fallback.
- `kv_namespaces` rows carry `(namespace_id, core_binding, global_flag, blob_threshold, policy_epoch, owner_scope)`.
- `global_flag` is designation only (D003): "global namespace" = a namespace whose *default address scope* includes central. It does not change resolution (still the named registry), and it **must not bypass the local-first read rule**: even in a global-flagged namespace, plain `get` reads the caller's local overlay first and only falls through on miss; only `--global` pins central (§3). A flag that silently reordered reads would be a second lookup path, which this design forbids.
- Content-blob identity inside any namespace is exactly the settled #9120 initial profile: unkeyed BLAKE3-256 over exact bytes, 32-byte digest (A-9120.1; research `part-a-addressing/research.md` #9120 row; implementation slices `part-a-addressing/implementation-slices.json` A9120-content-id; profile code in [content_id.rs](../../pkgs/id/crates/id-core/src/content_id.rs)). Fleet-kv neither redefines the hash nor invents a parallel digest (binding constraint shared with #9120's "neither this spec nor §3's may redefine the hash").
- Parse/URI transport, if ever exposed as `id://`, follows #9117 syntax transport only; this design mints no IDs and establishes no authority beyond the namespace's own write policy. Admission-gate semantics, if referenced, follow [admission_gate.rs](../../pkgs/id/crates/id-core/src/admission_gate.rs); fleet-kv defines its own grant rows (D014) and does not reinterpret the ID gate.

Coordination boundary: #9116 owns the registry model; #9117 owns URI syntax; #9118 owns the array core; #9120 owns the immutable content core. Fleet-kv is a consumer of all four and defines only the KV row envelope, the override/sweep rules, and the blob reference row below. It preserves Part A untouched: no redefinition of the hash, no new core, no parallel namespace grammar. Not this surface: the F12.5 in-world `kv`/`blob` providers (#1062: `kv.get/set/delete` with 8 KiB values / 1024 entries / 64 KiB per world) are world-sandbox ceilings, not the fleet CLI store — this design neither implements nor changes them.

**Key syntax and limits (explicit; the world-provider caps do not apply here).** Namespace names follow the `NamespaceId` rule (letter followed by letters/digits/hyphens, ≤ 63 bytes). Keys: 1–512 bytes, printable ASCII, `/` permitted as a hierarchy separator but no leading/trailing `/` and no empty segments. Inline value bytes: at most the effective threshold (§8). At most 1024 namespaces cached per host registry (policy, default in effect). Violations are typed refusals (`malformed_key`, `unknown_namespace`, `limit`), never silent truncation.

### 3. Read/list/set/delete/tombstone/revert semantics

Verbs: `fleet kv get|set|list` plus `delete`, `clear-override`, and `revert`. `--global` pins central for the verb it modifies; `--namespace NS` selects scope.

- **Read path (local-first, identity-scoped):** resolve the caller's `(owner, ctx, partition)` from the authenticated context; read the local overlay for the full tuple; on miss, fall through to the central head the caller's scope authorizes. A live local overlay row *overrides* the same key in central (D004). `--global get` skips local entirely and reads the central head at its current version — the authoritative bypass. `list` merges both scopes with origin marks (`local|central|both`, local winning display) and never silently hides the shadowed row — show it as `shadowed-by-local` / `shadowed-by-central` in verbose output.
- **Set path:** default writes the local overlay + enqueues outbox work; `--global set` writes central directly via CAS (authenticated; §9 for large values). Local sets carry the full identity tuple plus `(value-or-ref, base_version, writer_session, ts)`.
- **Delete — three distinct states, three distinct operations (D005):**
  1. `clear-override` — removes the local overlay row for the full identity tuple. Effect: the central value (if any) becomes visible again locally. Sweeps nothing (there is no overlay left to sweep).
  2. **Masking tombstone** (`delete`) — writes a local tombstone row for the tuple. Effect: the key reads absent locally and in merged `list` (shown with `masked` origin under `--include-tombstones`), while central is untouched until sweep. Sweeps as a mask: central records the tombstone under the sweeping identity's scope; other scopes' heads are unaffected.
  3. **History revert** (`fleet kv revert NS KEY --to <version>`) — appends a new version whose value equals the named prior version (new version number, full provenance). Operates on version history, never on the overlay: it does not clear overrides and does not mask central.
- **Freshness (states exactly which read is authoritative):** `--global get` reads authoritative central state at the returned head version. Plain `get` reads the local overlay if present, else the freshest central snapshot the host has pulled (snapshot timestamp shown). `sync` refreshes two things only: the cached namespace definitions (≤ 1 h staleness) and the central snapshot the host caches per namespace. Ordinary cached reads are at most **1 s** old (default in effect); `--no-cache` forces a central read. There is no background sync promise in v1.

### 4. Durable session-partition sweep

A session's local partition does not evaporate at disconnect (D006). On session end (and on explicit `fleet kv sync`), a sweeper merges the session's dirty rows into central under full-identity provenance:

1. Read session dirty set from `outbox` (durable, so a dead session still sweeps on next host run; disk-destroyed sessions recover from the survival archive, §1/§7).
2. For each key: `central_CAS(namespace, key, expected=base_version, new=row)` against `kv_heads`.
3. On success: write the idempotency receipt, then mark the outbox row done (order fixed: CAS → receipt+ack, §5). On conflict: run the §5 rule (versioned conflict, never silent last-writer-wins across sessions), append the conflict record, and leave both versions retrievable.
4. Masking tombstones sweep through the same CAS path as masks scoped to the sweeping identity.
5. Large values sweep by reference: the sweeper uploads through §9 first, then CASes the reference row. Central never accepts an unpublished blob reference.

Sweep is idempotent (outbox row ids + receipts make re-runs safe) and recoverable (crash between upload and CAS leaves a staged blob that staging TTL reaps, per #9084 component-F staging rules; crash between CAS and ack re-runs CAS, and the atomic receipt returns the original result instead of a spurious new conflict). "Two sessions sweeping the same key" is therefore a routine CAS conflict, not a special case.

### 5. CAS/versioned conflicts, retry, idempotence, recovery

Every mutable head uses compare-and-swap on `(namespace, key, version)` (D007), following the #9114 sidecar-head precedent (CAS on prior `(generation, snapshot_id)`; generation increments exactly once). There is no timestamp-only last-writer-wins anywhere in v1. Storage, not just assertions:

- `kv_heads`: one row per `(namespace, key)` — the current head `(version, value-or-published-ref, writer, ts)`.
- `kv_versions`: append-only; every CAS success appends `(namespace, key, version, parent_version, value-or-ref, writer, ts)`. Retention: full history while the key lives; tombstoned keys retain history (true-delete deferred to #9130 coordination).
- `kv_conflicts`: append-only; every failed CAS appends `(namespace, key, expected, actual, proposed_value, writer, ts)` and the loser receives the conflict id. "Both versions retrievable" reads: `kv get --version V` replays `kv_versions`; `kv conflicts NS KEY` lists conflict records.
- `idem_receipts`: `(outbox_id → {namespace, key, version, result})`, written **atomically with the CAS in one central transaction**. A replay of an already-applied `outbox_id` — even after the head has since advanced — returns the recorded result instead of attempting a new CAS (so it can never become a spurious new conflict).
- Retries are bounded with jitter: at most **5 attempts**, jittered exponential backoff capped at **1 s** per backoff (defaults in effect). Each attempt carries the outbox id as the idempotency key.
- Recovery order is fixed: outbox → upload (§9) → CAS+receipt (atomic) → ack. Any crash restarts at the oldest unacked outbox row. Staged-but-unpublished blobs are never referenced by any read path, so they are safe to reap by TTL.
- `concurrency_unsupported` is a typed error if some future adapter cannot CAS: fail closed, never degrade to silent overwrite (same posture as the V2 store seam in the Part A versioned design).

### 6. Five agents on one shared mid-size namespace (workload sketch + topology)

Workload assumed (planning fixture, stated so a later measurement can replace it — not a measured or free-tier guarantee): namespace holds ~10k–100k small keys (hundreds of bytes to a few KiB each; larger values are references per §8), five agents each holding the namespace open for minutes, read-heavy with intermittent writes (each agent writes ~1–10 keys/min, occasionally the same key family, e.g. status/claim rows). A "five agents × ~100 MiB" shape, where named, is the same kind of planning fixture, not a throughput promise.

- **Writer topology (D008): single-writer-per-key by CAS against the central primary.** There is no multi-master convergence in v1: all writers serialize per key on `kv_heads` CAS. KV CAS is not a queue lease: unlike the fleet queue's claim path ([claim.rs](../../pkgs/fleet/crates/fleet-queue/src/turso_queue/claim.rs)), there are no leases, no expiry, no steal — a failed CAS is a versioned conflict for a human or agent to resolve, never a grant that times out. Rows are not database grants: sharing a namespace grants nothing by itself; every write passes the namespace write policy at the pipeline layer.
- **Replica sync semantics:** no embedded replicas in v1 (D002 rationale, retained by D014 for shared namespaces). Agents read central directly (ordinary cached reads at most 1 s old, documented per namespace) or read a snapshot pulled at task start; `--global` bypasses the cache and reads the authoritative head. There is no background replica convergence to reason about, and therefore no divergence window beyond the stated cache age. Offline writes are permitted as **local intents**: they are ordinary overlay rows, fully usable locally, but they are not shared or committed until sweep CAS succeeds — "no offline fork" means no divergent shared history, not that offline work is forbidden.
- **Freshness:** reads state their source (`local|central@version|snapshot@age`); claim/status keys that need linearizability use `--global` reads + CAS writes. Nothing else gets linearizability, and the CLI says so.
- **Contention:** same-key write contention fails CAS fast (≤ 5 attempts, §5); hot keys should be claim/checkpoint-shaped rather than counter-shaped. No counters in v1 KV rows — a stored counter is a second copy of the truth and drifts (same argument as the [content-addressed media data-plane design](../../doc/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane.md) §6 against reference counting).
- **Cost:** central rows are small (values inline only at/below threshold; §8). Five agents polling `list` at 1 Hz against Turso HTTP is the abuse case: mitigate with the documented 1 s cache age, snapshot reads, and (if needed) a per-namespace read budget owned by the same policy epoch as the threshold. Not checked: per-request Turso billing for this workload — no billing claim is made here; measure on the fleet-kv Turso DB once Phase 2 lands.

Why not embedded replicas for the five-agent case: embedded-replica writes go to the cloud primary anyway (https://docs.turso.tech/features/embedded-replicas/introduction), so five replica-holders writing the same key still serialize centrally — replicas add a divergence/catch-up window without removing the central serialization point. v1 keeps that serialization explicit and auditable.

### 7. Host-local technology choice: SQLite vs embedded libSQL/Turso

D001/D014 decide: **plain SQLite (WAL) per host for local; Turso-hosted libSQL primary for shared central; no embedded-replica mode in v1.** Backup story: local = file snapshot + WAL + backup-manifest row (§1, numbers above); central = Turso's own durability plus the tombstoned, versioned row history (point-in-time story is the provider's, not a second log we maintain). Sandbox hosts ("sandbox has their own local store that maybe we back up and merge") get exactly the §4 sweep plus the §1 survival archive: their partition merges on session end, a dead sandbox's outbox merges on next host run, and a destroyed sandbox recovers from the archive — "merge" always means CAS merge, never file copy.

Alternatives steelmanned (challenge 2026-11-23):

- **Embedded libSQL replica as the local store.** Lost: writes still serialize on the cloud primary (Turso docs), so it buys a divergence window without removing central arbitration, and it needs network for the write path that session partitions need most. Revisit only with a convergence proof and an offline story.
- **Standalone Turso local-only (libSQL file, no sync) via the Rust SDK** (https://docs.turso.tech/sdk/rust/quickstart). Steelman: same-file durability as SQLite, libSQL API parity with the central primary (one driver, one SQL dialect), trivial later upgrade to a sync mode. Lost for v1 anyway: API sameness is not behavior sameness — the sync surface stays one connection flag away from silently becoming a replica, whereas plain SQLite (no sync primitives at all) makes "no sync" structural; and SQLite's backup API + WAL file story is the exact shape §1's snapshot needs. Cheapest to reason about wins for v1.
- **Turso Sync (`push`/`pull`) as the sync substrate.** Lost for v1: local-first and explicit, but conflict strategy is last-push-wins (https://docs.turso.tech/sync/usage, https://docs.turso.tech/sync/conflict-resolution), which contradicts the CAS-per-key invariant §5 needs for sweep/claim correctness. Possible later as a transport under CAS, not as the conflict rule.
- **Reference-only-everything (no inline threshold).** Lost: for small KV values the reference machinery (staging, capability mint, GC sweep) costs more than the bytes it saves — the same argument component F makes for inline small content, applied one tier down in size.
- **Copy-bytes-into-central-DB (no object plane).** Lost: prices media as database, abandons dedup, and makes publication a divergent copy — component F's evaluation already rejects this for content above threshold, and KV values above threshold are the same case.
- **Bearer-hash reads (hash grants bytes).** Rejected outright: hashes are enumerable and leak by design; serving on hash presentation alone fails the access-control soundness axis of #9084. Never the default; not even a flag.
- **Timestamp last-writer-wins for sweep conflicts.** Rejected: loses concurrent sessions' work silently and reintroduces ABA the #9114 head rule already forbids. CAS conflict with both versions retrievable is the floor.

### 8. Arbitrary bytes, streaming, blob threshold knob

Values are arbitrary bytes with a streaming interface (no full-buffer requirement on push/pull above a small in-memory cap). The threshold knob (D009):

- **Per-namespace policy value** (`kv_namespaces.blob_threshold`, bytes), default **256 KiB** — inside Drew's named shape (250 KB / 500 KB / 1 MB / 1 GB) at its low end. This default is subordinate to the shared per-partition placement policy, not a competing protocol: where KV bytes and #9084 component-F content share a partition, component E owns the threshold and the 16 KiB conservative starting default ([media data-plane design](../../doc/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane.md) §Recommendation) governs placement until #9098 measurements move it. The **effective threshold is a hard inline ceiling**: `effective = min(namespace knob, partition ceiling)`. Changing either changes placement of new content only, never identity or access (policy, never protocol).
- **Per-push override** (`--force-blob`, exact flag name owned by Phase 1): one direction only — force blob treatment at or below the effective threshold (e.g. a small value that should not live in the row store). There is no force-inline override: nothing may place bytes inline above the effective ceiling. The earlier `--inline-max` both-directions sketch is withdrawn as a fatal ceiling conflict.
- At or below the effective threshold: bytes inline in the row. Above: §9 reference path. Promotion (inline → referenced) is a new version with identical read semantics; no client-visible change.

### 9. Trusted Cloudflare endpoint, supported credential path, reference-based read (no bearer hash)

Auth uses only credentials the hosted front door actually validates (D011, corrected after review — the earlier draft cited the secrets-broker fetch path, which is withdrawn):

- **Push path:** the pusher authenticates to the trusted Cloudflare endpoint with either (a) an RP/agent JWT validated as in [auth.ts](../../pkgs/fleet/hosted-worker/src/auth.ts) lines 140–184 (issuer/audience check, subject mapped to a tenancy `bindingId`; agent credentials must carry lane/session claims), or (b) a route-scoped service token validated at lines 187–201 with `authorizeRoute` scoping it to the kv-upload door (lines 206–210). The endpoint then checks the namespace write policy for `(binding_id, context, namespace)` and only then accepts staged bytes. The content hash travels as an *assertion to be verified*, never as authority. The [fleet-secrets](../../pkgs/fleet/crates/fleet-secrets/src/lib.rs) broker-first resolution (lines 15–28) plus [fleet-gh-credentials.md](../../pkgs/fleet/docs/fleet-gh-credentials.md) is a named-secret *fetch* protocol and is not a request-authentication mechanism anywhere in this design.
- **Read path:** a bearer hash grants nothing (binding rule shared with the [media data-plane design](../../doc/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane.md) §5 "the hash is not a capability"). Readers resolve their KV row authorization first (local overlay or central head, per namespace policy + authenticated context), then receive a **short-lived (5 min, default in effect), object+operation-scoped capability** minted from live reference state; the adapter serves bytes only under that capability, with replica fallback during placement movement (same contract as the [self-hosted tier adapters design](../../doc/2026-11-21T21-40-00Z_design_self_hosted-tier-adapters/2026-11-21T21-40-00Z_design_self_hosted-tier-adapters.md) object adapter contract).
- **Upload safety:** `put_staged → verify (length + BLAKE3-256 recomputed on receipt, streaming) → publish`. Partial uploads are staging state, never accepted objects (same invariant as A-9120.1 and component-F staging). Multipart completion is not publication; only the service verification step makes the reference row writable.
- **GC safety:** mark-and-sweep per partition exactly as component F §6. The authoritative reference state the sweep reads is the set of **published** rows in `kv_heads`/`kv_blobs` (intermediate `staged`/`verified` states live only in the endpoint's staging area, never in central rows — central accepts `published` only). The sweep shares component F's per-partition serialization point concretely: KV reference creation (publish-CAS) and the GC delete fence are transactions against the same central primary, ordered by the per-partition sequence the primary assigns. A reference created during candidate/grace/re-verify races is thereby ordered: a publish-CAS landing before the fence's re-verify is seen and clears candidacy; one landing after finds no verified replica under the fence and the writer re-stages from its held bytes (a re-upload, not data loss). Per-jurisdiction dedup partitions from #3401 bound every sweep (D012); the derived index is a cache; re-verify at grace expiry; epoch-fenced delete; audit event per delete.

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
  state: published                    // central rows accept published only (§9)
}
```

No new hash, no new multicodec, no bearer URL persisted as identity. The locator is adapter-private (component-I peer principle: no client-visible branch on backend kind).

### 11. Lists and tables: deferred (D015; D013 reversed/superseded)

Phase 3 is blob-only. List/table semantics are not designed here and no manifest/table implementation ships in this plan. What this design reserves — so a later shapes proposal arrives without a second engine — is a versionable envelope only:

- The KV row envelope carries an envelope version (`kv:raw-v1` for inline bytes, `kv:ref-v1` for a `KvBlobRefV1` row). Unknown envelope versions are refused, not ignored.
- Any future list/table shape must be a new envelope version (or a new key convention) proposed with its own versioning/upgrade rule, CAS/tombstone/sweep inheritance statement, and threshold/GC interaction — none of which is fixed here.
- Explicit non-goal restated: no SQL surface (#10359); when lists/tables are later proposed, composition precedents they must address include append semantics, ordering, and column types — deferred, not decided.

## Decisions

Source of every entry below is agent-preliminary; status of the record is preliminary. Work proceeds on these defaults; implementation evidence (not a gate) owns tuning — see `human-review.md`.

- **D001 — Local store is plain per-host SQLite (WAL), not embedded libSQL/Turso or libSQL-file, in v1.** Source: issue #10359 item 2 + backup question; evidence: offline-write and file-backup needs (§1, §7); Turso embedded-replica write path (writes to cloud primary; https://docs.turso.tech/features/embedded-replicas/introduction); standalone local-only steelman via Rust SDK (https://docs.turso.tech/sdk/rust/quickstart), rejected §7. Status: preliminary (retained with challenge refinement 2026-11-23). Reversal cost: medium (replaces local engine + backup story; central protocol unchanged).
- **D002 — [REVERSED 2026-11-23, superseded by D014] Central store is one Turso-hosted libSQL primary; per-group scopes are row-level in v1.** Reversal reason: read as either/or vagueness ("provisioning scopes") and an unproved rows-equal-grants posture. Retained content moved into D014 with an explicit grant mechanism and fail-closed policy. Do not cite D002 as active.
- **D003 — Global namespace = designation flag on the distributed registry; resolution always names a registry; the flag never bypasses local-first reads.** Source: #10359 item 3; aligns #9116 registry under the [namespace_definition.rs](../../pkgs/id/crates/id-core/src/namespace_definition.rs) local-registry constraint. Status: preliminary. Reversal: low.
- **D004 — Local-first, identity-scoped reads with local-override; `--global` pins central; merged list shows shadowed rows.** Source: #10359 item 2 read path. Status: preliminary. Reversal: low.
- **D005 — Three delete-related states: clear-override, masking tombstone, history revert.** Source: sweep-merge need (#10359 item 4) + #9115 tombstone reconciliation precedent. Status: preliminary. Reversal: medium (resurrection bugs if collapsed).
- **D006 — Sweep merges full-identity partitions via durable outbox + CAS, with a 15-minute survival archive.** Source: #10359 item 4 + context-propagation note filed the same day. Status: preliminary. Reversal: medium.
- **D007 — All mutable heads are CAS-versioned with append-only version/conflict history and atomic idempotency receipts; no timestamp LWW.** Source: #9114 head rule (CAS on prior generation); store-seam `concurrency_unsupported` posture. Status: preliminary. Reversal: high (correctness invariant).
- **D008 — Five-agent shared namespace: single-writer-per-key CAS on central; ordinary cached reads at most 1 s old; `--global` authoritative bypass; offline work is local intent until sweep commits; KV CAS is not a queue lease.** Source: #10359 item 5 question; evidence: claim-path precedent ([claim.rs](../../pkgs/fleet/crates/fleet-queue/src/turso_queue/claim.rs)) distinguished, not inherited; legacy embedded-replica write path (writes to cloud primary; https://docs.turso.tech/features/embedded-replicas/introduction) and Turso Sync last-push-wins (https://docs.turso.tech/sync/usage, https://docs.turso.tech/sync/conflict-resolution) both noted and rejected as the v1 conflict rule. Retry: ≤ 5 attempts, jittered exponential backoff capped at 1 s. Status: preliminary (retained with challenge refinement 2026-11-23). Reversal: medium.
- **D009 — Effective blob threshold is a hard inline ceiling (`min` of the KV per-namespace knob, default 256 KiB, and the shared partition ceiling); per-push override forces blob earlier only.** Source: #10359 threshold question; precedent: component-F "threshold is policy, never protocol" with 16 KiB conservative start. Status: preliminary. Reversal: low (policy value).
- **D010 — Blob identity is exactly #9120 `raw-blake3-256` over exact bytes; reference row format `KvBlobRefV1` reuses that profile; central accepts `published` only.** Source: #9120 A-9120.1; profile code in [content_id.rs](../../pkgs/id/crates/id-core/src/content_id.rs). Status: preliminary. Reversal: high (identity change).
- **D011 — Endpoint auth is RP/agent JWT or route-scoped service token per [auth.ts](../../pkgs/fleet/hosted-worker/src/auth.ts); the secrets-broker fetch path is not request auth; hash is never a bearer credential; reads mint 5-minute scoped capabilities.** Source: hosted auth ([auth.ts](../../pkgs/fleet/hosted-worker/src/auth.ts) lines 140–210); component-F §5 access rule. Status: preliminary. Reversal: high (auth architecture).
- **D012 — Blob GC is component-F mark-and-sweep per `(jurisdiction, tier)` partition sharing the central per-partition serialization point; kv published rows are the authoritative reference set.** Source: component-F §6 + #3401 partitions + #9137 priority note. Status: preliminary. Reversal: medium.
- **D013 — [REVERSED 2026-11-23, superseded by D015] Lists/tables as typed manifest compositions.** Reversal reason: over-designed ahead of the shapes it depends on; Phase 3 is blob-only. Do not cite D013 as active.
- **D014 — Hybrid central placement: shared primary by default; group databases by explicit grant only.** Shared namespaces live on the one fleet-kv primary. A group database exists iff a `kv_grants` row `(group, database, allowed namespaces, allowed ops, policy_epoch)` authorizes it under a fail-closed admission/provisioning policy; no default per-session proliferation. v1 operational integration (migrations, credential wiring, grant policy implementation, backup/restore runbook, health probes) is required Phase 2 scope, not assumed. Source: challenge outcome 2026-11-23 (investigator `ses_e5df780c496ffe06q0f9UZRHmj`) + #10359 "own small database" note bounded by tenancy. Status: preliminary. Reversal: high (splits the primary where grants exist).
- **D015 — Versionable envelope only; list/table semantics deferred.** The KV row envelope is versioned (`kv:raw-v1`, `kv:ref-v1`); future shapes arrive as new envelope versions with their own proposals. No manifests or tables in this plan; Phase 3 is blob-only. Source: challenge outcome 2026-11-23. Status: preliminary. Reversal: low.

## Challenge record (2026-11-23)

D001/D002/D008/D013 were reopened under readonly investigator `ses_e5df780c496ffe06q0f9UZRHmj`, whose final read was adopted via the parent session. Outcomes (this revision):

- D001: retained with refinement — standalone Turso local-only steelmanned (one driver/dialect, trivial sync upgrade) and rejected (one flag from silent replica; SQLite makes no-sync structural). No false dichotomy: three local options are now on record.
- D002: reversed, superseded by D014 — the either/or ("provisioning scopes") is replaced by the hybrid: shared primary default + explicit-grant group DBs under fail-closed policy. Rows-are-not-grants stated explicitly. No default per-session proliferation; no pricing claims (Not checked: Turso per-DB pricing/org limits).
- D008: retained with refinement — retry bounded (≤ 5 attempts, jittered exp backoff, 1 s cap), ordinary cache age ≤ 1 s, `--global` authoritative bypass stated, KV≠queue-lease distinction recorded. Not checked: Turso per-request billing under the five-agent probe.
- D013: reversed, superseded by D015 — Phase 3 blob-only; envelope versions only, no manifests/tables now.
- None of the four outcomes needed a human tossup; all proceed as defaults with implementation evidence owning tuning.
- Re-challenge of the reopened choices is expected after this repair; outcomes will be recorded here with dates.

## Alternatives considered

See §7 for the local-store alternatives (embedded replica, standalone local-only, Turso Sync). Remaining:

- **Per-session Turso DB as the default unit of sync.** Rejected: matches Drew's "handed its own small database" intuition but multiplies primaries without settling per-key arbitration and with unproved pricing/ops cost. D014 keeps the shared primary default; grants are the only path to a real group DB.
- **Reference-only-everything (no inline threshold).** Lost: for small KV values the reference machinery costs more than the bytes it saves.
- **Copy-bytes-into-central-DB (no object plane).** Lost: prices media as database, abandons dedup, makes publication a divergent copy.
- **Bearer-hash reads.** Rejected outright: enumerable, leak-by-design hashes as capabilities fail #9084 soundness. Never the default; not even a flag.
- **Timestamp last-writer-wins for sweep conflicts.** Rejected: silent loss + ABA. CAS conflict with both versions retrievable is the floor.

## Consequences

- Gets easier: session work product survives disconnect by construction (15-minute archive bound); namespaces stay one system with ID (#9107 family) instead of a fleet-private dialect; large values stop pricing as database rows; every conflict is a retrievable record, not silent loss.
- Gets harder: every writer implements CAS retry; every large push does staged→verified→published instead of a single write; reads that need linearizability must say `--global` explicitly; GC/tombstone literacy becomes operator knowledge.
- Must stay compatible: #9120 hash profile bytes (never rehash/reinterpret); legacy local state (adopt-or-CAS-merge, never overwrite); supported-credential auth only (no new mechanism smuggled in via the secrets path); adapter neutrality (R2 vs RustFS invisible above the adapter, per component I); the ID signed-journal source-of-truth contract (fleet outbox claims no ID-world authority).

## Open questions

- **O1 — `--global` surface exactness:** flag on every verb vs `fleet kv global <verb>` subcommand. Default in effect: flag (`--global`), matching the issue's sketch. Settles: Phase 1 build.
- **O2 — Threshold/ceiling measurement.** Defaults in effect (256 KiB knob, 16 KiB partition start). Settles: measurement (#9098-style KV workload) — evidence, not a gate.
- **O3 — List/table shapes.** Deferred to a later proposal (D015); the envelope versions here are the only seam fixed.
- **O4 — True-delete/retention interplay** with #9130-versioning/#3401 erasure flows for tombstoned KV rows. Deferred to plan; masking tombstones are compatible with either outcome.
