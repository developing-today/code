# What is left in the design: decisions, open questions, and status

- Date: 2026-10-11 (rewritten after round 2 research and the owner's decisions)
- Purpose: the single list of decisions, open questions, and remaining work. Each row says where its detail lives.
- Status terms: **Decided** (the owner accepted it), **Changed** (the owner or round 2 research changed the earlier answer), **Owner question** (waiting for an answer, listed in section 2), **Open** (needs a step in the plan), **Deferred** (explicitly later).

## Start here

1. [Phased plan](./plan-phases.md): eight phases, the true blockers, the parallel tracks, the testing layers, and the rules every phase follows.
2. [Explainers](./explainers.md): ChainExt, waiting caps, storms and merges, epochs and history, range versus LogSync, compaction, scheme versions, tickets, ed25519, storage, relays.
3. [BeeKEM assessment](./beekem-assessment.md): why BeeKEM is not adopted now, and when to reconsider it.
4. [Round 2 results](./research-round2/README.md): the eleven research answers, with their evidence.
5. [Round 1 spikes](../2026-10-11T03-44-33Z_design_groups_events_delegation/spikes/README.md): S1 to S12, C1, C2 and BeeKEM.

## 1. Decisions (the original 19, and the new answers)

| #   | Decision                               | Status  | Answer, or what is still open                                                                                                                             |
| --- | -------------------------------------- | ------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Dependents of voided events            | Decided | Authority-based, as p2panda's StrongRemove does                                                                                                           |
| 2   | Transfer on removal                    | Decided | Derived from the signed removal                                                                                                                           |
| 3   | Use-count bounds on world capabilities | Decided | Dropped from the protocol; local enforcement only                                                                                                         |
| 4   | Content on removal                     | Decided | Content is kept. Removal stops new content                                                                                                                |
| 5   | `observe` gating on blobs              | Decided | Each request is gated by the world's fetch rules. Observe is off by default                                                                               |
| 6   | Waiting caps                           | Decided | 256 per author and 7 days, configurable per world                                                                                                         |
| 7   | Joiner history                         | Changed | `full` by default. `recent:N` per world. Presets 32 and 128 (owner question 5)                                                                            |
| 8   | `scheme: none` notice                  | Decided | Explicit notice in the UI and in group metadata                                                                                                           |
| 9   | Large-group gate                       | Decided | No gate. Warnings only, at 96, 128 and 1,000 members                                                                                                      |
| 10  | Invite style                           | Decided | Bearer link (Seitan V2), with expiry and single use. Typed key optional with approval                                                                     |
| 11  | Pull default                           | Decided | Open pull, with an authorized option                                                                                                                      |
| 12  | Our relay                              | Changed | No Tunnel account now. The n0 relay is used, and users see the server's public IP. Later: a local server behind a Cloudflare Tunnel, and VPS for testing  |
| 13  | Relay access control                   | Decided | None now. Revisit at implementation                                                                                                                       |
| 14  | mDNS default                           | Changed | On for `serve` (`--no-mdns` turns it off). Off in the library. The earlier design said off                                                                |
| 15  | DHT default                            | Decided | Off. Opt-in per world                                                                                                                                     |
| 16  | Ticket policy                          | Changed | Regular tickets work. Addresses are allowed when needed. Signatures are optional (option E)                                                               |
| 17  | Fork recovery                          | Decided | Removal and re-enrolment under a new device key. The alternative, quarantine with approval, is elaborated in task 10 and is a later option                |
| 18  | Upstream patch policy                  | Decided | Patches stay in our fork (developing-today). Nothing goes upstream. No upstream issues or PRs. The scheme v1 change is in the same fork                   |
| 19  | ed25519 version                        | Changed | Round 2: stay on 3.0.0 with a conversion shim, and switch to `verify_strict`. Moving to 2.x is not possible while iroh 1.3.0 is pinned (owner question 1) |

## 2. Owner questions (answers needed before Phase 0 starts)

The plan lists the preferred answer for each in its section 4. Short form:

1. **Ed25519.** Stay on 3.0.0 with the shim, and use `verify_strict` [preferred].
2. **Scheme v1 as the first release.** The serde_bytes encoding ships first, since nothing has shipped [preferred].
3. **Unreadable invite event.** When the invite event cannot be read before joining, proceed unverified with a label [preferred], or refuse.
4. **Welcome frame cap.** A chunked welcome above one identity frame cap [preferred], or the 1 MiB world-session cap.
5. **`recent:N` presets.** 32 for quiet worlds and 128 for churn-heavy ones [preferred]. `full` stays the default.
6. **Clippy 1.99.** Allow the two new lint names for now and keep a TODO [preferred], or fix the about 50 sites in Phase 0.
7. **Document location.** `doc/` at the repo root, plus `.opencode/plans/` copies of plans [preferred]. `pkgs/id/AGENTS.md` names `docs/`.
8. **Real-network tests.** Wait for the VPS and networks you plan to set up [preferred].

## 3. New items found in round 2 (each has a step in the plan)

| #   | Item                                                                                                                                                | Source                  | Plan step               |
| --- | --------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------- | ----------------------- |
| N35 | A fork inside a range-sync batch aborts the batch and loses unrelated events                                                                        | range-config            | P3.1a                   |
| N36 | T=0 never terminates on a mismatched key. Validate K and T                                                                                          | range-config            | P3.1b                   |
| N37 | Range flights above about 524,000 IDs exceed the 16 MiB frame limit. Chunk to a budget                                                              | range-config            | P3.1c                   |
| N38 | p2panda-sync depends on p2panda-store with its SQLite default. Fork the one dependency line, or implement LogSync ourselves                         | store-fit, explainers 5 | P3.2                    |
| N39 | `pkgs/id/default.nix` runs `rustup default 1.89.0`, so the Nix check ignores the pin                                                                | toolchain               | P0.12                   |
| N40 | Five plain `verify` calls must become `verify_strict`, with regression tests                                                                        | ed25519                 | P0.11                   |
| N41 | A `recent:N` joiner: pre-welcome history must stay outside the crate. Delivering it failed 80 of 120 welcome runs (agent-reported; reproduce first) | welcome-recent          | P4.5                    |
| N42 | Welcome size budget and chunking above the frame cap                                                                                                | welcome-recent          | P4.12                   |
| N43 | iroh probes only the first five relays for QAD. QAD-capable relays must come first                                                                  | cloudflare-workers      | P3.6                    |
| N44 | The web identity store moves from libsql to redb, with AES-GCM values                                                                               | turso                   | P1.9, P1.10             |
| N45 | First creation of the store file: create under a temporary name, commit, rename, fsync the directory                                                | crash-tests             | P1.3                    |
| N46 | Clippy 1.99 lints and the lints review                                                                                                              | toolchain               | P8.8 (owner question 6) |
| N47 | Lazy full history for `recent:N` joiners                                                                                                            | explainers 4            | P4.6                    |
| N48 | Welcome determinism: sort before signing or hashing                                                                                                 | scheme-serde            | P4.4                    |
| N49 | Debug and release builds recover different valid prefixes at four boundary points. Explain it. If it is a redb bug, revisit the engine              | crash-tests             | P1.4                    |
| N50 | Identity: `check_target` does not trim pasted text                                                                                                  | ticket-signing          | P6.3                    |

## 4. Remaining open questions, with the preferred answer

| #   | Question                           | Preferred                                                                                | Alternatives                                                  |
| --- | ---------------------------------- | ---------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| N1  | Storage engine                     | redb everywhere (Turso rejected by the spike)                                            | Turso, if its performance problems are fixed upstream         |
| N2  | Identity store                     | redb with AES-GCM values                                                                 | Keep libsql. Turso (smaller diff, experimental encryption)    |
| N3  | p2panda-store                      | Traits over redb, no fork now                                                            | Fork p2panda-store and replace its SQLite part                |
| N4  | Chain identity                     | ChainExt on every event                                                                  | One log per group                                             |
| N5  | Sync method per session            | Range above the 3.5% crossover, LogSync below                                            | Range always. LogSync always                                  |
| N6  | Range parameters                   | K and T local, validated, no wire change                                                 | Protocol-negotiated K and T                                   |
| N7  | Periodic rotation                  | Off. Rotate on removal and explicit update                                               | Rotate every N events, or weekly                              |
| N8  | Recovery from a compromised device | Manual remove and re-add, as a CLI procedure                                             | Automatic on a detected key change                            |
| N9  | from_join membership fix           | Keep the fork (decision 18). Revisit moving it into our DGM                              | Our DGM's `from_welcome` inserts the joiner, with no fork     |
| N10 | serde_bytes                        | Scheme v1 (owner question 2)                                                             | Stay on stock (3.66 times the welcome size)                   |
| N11 | Warning thresholds                 | 96, 128, 1,000                                                                           | Other values from the cost data                               |
| N12 | BeeKEM                             | Not adopted now                                                                          | Adopt when the conditions in its assessment are met           |
| N13 | Storage power-loss testing         | In-process gate in CI. LazyFS as an optional Linux job                                   | VM power cut for final sign-off                               |
| N14 | p2panda source                     | Git rev `3e84cac`                                                                        | crates.io 0.7.x with patches                                  |
| N15 | Toolchain                          | 1.99.0                                                                                   | Stay on 1.97.0                                                |
| N16 | `recent:N` default N               | 32, with 128 as the churn-heavy preset                                                   | 8, or 1                                                       |
| N17 | `store.rs` doc comment             | Fix it: FsStore is redb                                                                  | none                                                          |
| N18 | Observe default                    | Same as decision 5                                                                       | none                                                          |
| N19 | Tunnel account                     | None now. You create any account yourself                                                | Later, with your go                                           |
| N20 | Scheme v1 mechanism                | A fork change of two lines (owner question 2)                                            | The transcoder on the outer field (76.9 B per epoch, no fork) |
| N21 | `recent:N` implementation          | Adapter calls `Dcgka::add` directly. `EncryptionGroup::add` always sends the full bundle | Patch the high-level add                                      |
| N22 | Heads rule                         | Cap 32, the author's latest always named, merge on overflow                              | Cap 16 (162 merges per storm), or cap 2 (3,159)               |
| N23 | Freshness window                   | 5 minutes, configurable per server, world and inviter                                    | A wall-clock window only                                      |
| N24 | Ticket signing                     | Option E: standard paste, signed invite event checked when present                       | Option B (our envelope; standard clients refuse it)           |
| N25 | Relay operation defaults           | SIGINT, config required, metrics on loopback (applied in the kit)                        | Keep SIGTERM (not recommended)                                |
| N26 | BeeKEM for groups above 1,000      | Not adopted now. DataScheme everywhere, with warnings                                    | Fix and extend BeeKEM ourselves                               |
| N27 | Bulk catch-up parameters           | K=4, T=8 locally, for bulk catch-up only                                                 | K=16 always                                                   |
| N28 | Migrating DataScheme groups        | Not needed while nothing has shipped                                                     | A scheme switch at a barrier, later                           |
| N29 | QAD source                         | n0 default, verified on real networks                                                    | A QAD VPS first                                               |
| N30 | Address exposure                   | Documented. "Authorize before connecting" is access control, not privacy                 | Relay-only transports, or a patched iroh                      |
| N31 | Relay capacity for fallback pairs  | Plan for symmetric-to-symmetric pairs falling back                                       | Accept lower direct-path rates                                |
| N32 | Startup wait on relays             | Wait only for the home relay, with a deadline                                            | Wait for every entry                                          |
| N33 | Tag compaction                     | Snapshot at a causal cut, then the tail                                                  | No compaction                                                 |
| N34 | Churn warnings                     | Warn on re-add churn as well as size                                                     | Size only                                                     |

## 5. Round 2 results (eleven questions, one folder each)

| Folder               | Question                                                  | Verdict                                                                                  |
| -------------------- | --------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `ed25519`            | Versions, compatibility, the newest release               | 3.0.0 is newest. 2.x and 3.x are wire-compatible. Stay on 3.0.0 with a shim              |
| `turso`              | Turso versus redb for the event store, and libsql removal | redb wins on every measure. Identity moves to redb                                       |
| `crash-tests`        | LazyFS and a portable crash test                          | In-process model: 7,808 runs, zero failures. LazyFS optional (user namespace only)       |
| `toolchain`          | Newest stable, flake update, MSRV                         | 1.99.0. No lock update needed. The Nix check pins 1.89.0 (a bug)                         |
| `range-config`       | Are K and T on the wire; LogSync crossover                | Local only. Crossover is 3.5% of chains, not 40. Fork-in-batch bug found                 |
| `scheme-serde`       | Scheme versions and the welcome encoding                  | v1 (serde_bytes) is 3.66 times smaller, and the only option that reaches the floor       |
| `iroh-allowlist`     | Can iroh refuse peers before it exposes addresses         | No: identity is known only after the TLS handshake. It is access control, not privacy    |
| `ticket-signing`     | Signed tickets without breaking regular ones              | Option E. Regular paste works, and signed events are checked when present                |
| `welcome-recent`     | What `recent:N` and `full` cost, and what 32 means        | 32 is 5.3 KB per join (stock). `full` grows linearly, and re-adds grow quadratically     |
| `store-fit`          | Implementing the store traits over redb                   | About 650 to 900 lines plus tests. No fork needed. LogSync pulls SQLite (N38)            |
| `cloudflare-workers` | Later relay options                                       | Tunnel carries no QAD. QAD probes five relays. Workers can serve pkarr HTTP and DoH only |

## 6. Spikes (round 1, all done)

| Spike                        | Verdict                                                                                        |
| ---------------------------- | ---------------------------------------------------------------------------------------------- |
| S1 to S6, C1                 | Done. Earlier results stand, with the corrections recorded in round 2                          |
| S7 range sync on real stores | Works and matches S1. Keep K=16 and T=16 as defaults                                           |
| S8 observed-remove tags      | Hold. The clock covers the causal past. Compaction needs a snapshot                            |
| S9 Seitan V2                 | Derivations match Keybase's vectors. Verify the accept bytes and admin handler before shipping |
| S10 heads orderer            | Cap 32 needs the merge rule and the own-latest rule                                            |
| S11 relay operation          | Restart is survived. Fix SIGTERM. Metrics on loopback                                          |
| S12 welcome size             | 146.6 B per epoch stock, 40.0 B with serde_bytes                                               |
| C1 t2, t4 re-run             | Both pass                                                                                      |
| C2 holepunching              | Full cone and port-restricted succeed. Symmetric pairs fall back. QAD is required              |
| BeeKEM                       | Not adopted now. See its assessment                                                            |

## 7. Remaining work by task doc

| Task                                          | Area                                                          | Status                               | Plan phase                        |
| --------------------------------------------- | ------------------------------------------------------------- | ------------------------------------ | --------------------------------- |
| [01](./task-01-phase0-fixes.md)               | Phase 0 fixes: tuple (done), pull bytes, MSRV, dial deadlines | Partly done                          | Phase 0                           |
| [02](./task-02-events-and-storage.md)         | Events, storage, caps, crash gate                             | Designed; decisions made             | Phase 1                           |
| [03](./task-03-membership-and-delegation.md)  | Membership, delegation                                        | Designed                             | Phase 2                           |
| [04](./task-04-sync-blobs-transport.md)       | Range and LogSync, blobs, `observe`                           | Designed; decisions made             | Phase 3                           |
| [05](./task-05-encryption-and-history.md)     | Scheme v1, history, heads, warnings                           | Designed; decisions made             | Phase 4                           |
| [06](./task-06-tags-documents-search.md)      | Tags, documents, search                                       | Designed; S8 done                    | Phase 5                           |
| [07](./task-07-invites-and-pull.md)           | Seitan V2, invites, pull                                      | Designed; decisions made             | Phase 6                           |
| [08](./task-08-connectivity-extensions.md)    | Relays, tickets, mDNS                                         | Designed; decisions made             | Phases 3 and 7                    |
| [09](./task-09-time-and-witnesses.md)         | Time and witnesses                                            | Deferred                             | Phase 8                           |
| [10](./task-10-open-design-items.md)          | Retention, multi-device, fork recovery, editing, views        | Designed; preferred answers recorded | Phase 8 (and 6 for fork recovery) |
| [11](./task-11-dependencies-and-upstream.md)  | Upstream and forks                                            | Decisions made                       | Phase 0                           |
| [12](./task-12-dependencies-and-toolchain.md) | Toolchain and dependencies                                    | Decisions made                       | Phase 0                           |

## 8. Known discrepancies in the current code

- `pkgs/id/default.nix` runs `rustup default 1.89.0`, so the Nix check ignores `rust-toolchain.toml` (N39).
- `pkgs/id` declares a Rust version that is too low. p2panda needs 1.96 (P0.5).
- `src/store.rs` says FsStore is SQLite. It is redb (N17).
- `serve` enables mDNS by default, and the connectivity design says off. The decision is now on (decision 14), so the design text changes, not the code.
- The connectivity design says peers re-home in 20 to 22 s. Three runs measured 20.5, 21.6 and 22.5 s, so it should read 20 to 23 s.
- `spikes/shared/c1-connect/RESULTS.md` has stale lines (141 and 144). A dated re-run note is appended.
- `pkgs/id/AGENTS.md` says plans go in `.opencode/plans/` and docs in `docs/`. The repo's design docs are in `doc/` (owner question 7).
- The Roc test, the world-module hang, and the three `world_compile` tests are pre-existing failures (P0.1).
- Clippy: a target directory must use one rustc. The Nix clippy and the rustup clippy disagree, which gave E0514 errors.

## 9. TODO (later)

- **Clippy and other lints.** The owner noted a 9k-error lint output and asked for a review later. Nothing is done now. The review covers the 141 pre-existing `pkgs/id` lints under `-D warnings`, the 1.99 additions, and the lints in other tools. Plan step P8.8.
- **Lazy history, the `recent:N` default, and the welcome frame cap** are waiting on owner questions 4 and 5.
- **Real-network tests** wait for the VPS and networks (owner question 8).
- **Merge and push.** The feature branch merges to `main` only when the owner signals (the owner's instruction). The merge of `main` into the feature branch is committed locally as `d702af03`, and is not yet pushed.

## 10. Evidence and locations

- Repo design folder: `doc/2026-10-11T03-44-33Z_design_groups_events_delegation/`.
- Remaining-work folder: `doc/2026-10-11T11-08-42Z_design_remaining_work/`.
- Research round 2: `doc/2026-10-11T11-08-42Z_design_remaining_work/research-round2/` (copies of each folder's results, logs and harness code, without vendored crate sources).
- Raw research workspace (outside the repo): `/home/user/work/research-round2/`.
- Spike workspace (outside the repo): `/home/user/work/spikes-next/`.

## Owner decisions on groups and friendship (2026-10-12)

- Anyone may create a group.
- A normal group makes its creator Admin.
- A friendship is a group template (member_cap 2, mutual request and accept, dissolve on removal). It has no Admin by default. A config option at creation can make both friends Admin.
- If the friends are Admin, either one can change the other's role afterwards.
- Changing a group's size later is a policy change by an Admin (member_cap is a policy field).
- Friendship is therefore built on the group model, not as a standalone pair module. The earlier friendship module (P2.4, per-account cap 100) is superseded.

## Owner decisions on group creation, provisioning, and names (2026-10-12)

- Anyone may create a group by default. A server config can restrict creation to listed users, to members of listed groups, and optionally to a minimum role level.
- A server config can create users, create groups, and add members and roles by user ID.
- Server config keys name users and groups locally. The server translates each key to its user or group ID at load time. An unknown key fails startup.
- Groups have an automatic ID (BLAKE3 of the create event) and an official name (a field an Admin changes). Aliases are tag claims: any number per group, each attributed to the key that added it.
- Users and groups are taggable with the same tag mechanism used for documents and bytes. A group's subject is its create event ID. A user's subject needs a decision (see below).

Open for the owner:

1. User subject for tags: the user's first identity event ID, or the account key itself?
2. Two groups claim the same alias: show both as contested, never pick one silently (recommended). Does a server config key always win locally?
3. Official names unique per server, or not (recommended: not; aliases and config keys disambiguate)?

## Owner decisions on aliases (2026-10-12)

- Aliases are tag claims on the object. Each alias shows its text, its creation date, and the account that made it.
- Two or more aliases on one object are shown as contested. Users can inspect the full set. Nothing picks one silently.
- A server config sets a primary alias, separate from the name. Until the server sets one, the server may show its own alias as a fallback (meaning to confirm).
- Later: sort methods (creation time, votes or popularity, a user's chosen perspective). Per-user or per-server preferred and primary aliases. Display shows the name, the preferred alias, and the other aliases.
- Later: users can view aliases and set one as their user alias, which is the same thing as a pet name. Whether the user alias is shown by default is open.
- Later: a group can act through its members. A group can make a tag or upload a document. The acting user is always attached, and a user can act as the group.
- Strong removal of a user does not delete their data. It sets markers that invalidate that user's authority from the removal point.
- No official names and no uniqueness. Only event and object IDs are unique. Users, groups, tags, and aliases may share names.
- Disambiguation uses the shortest unique prefix of an object's ID.

Open for the owner:

1. Tag subject for a user: the first identity event ID (recommended), or the account key. Hashing the account key adds no privacy, because keys are already public.
2. Server alias fallback: confirm that it shows only until the server sets its primary alias.
3. Show the user alias by default, or only the name?
4. Prefix or suffix for disambiguation: prefix (recommended), since prefixes sort and match like git short IDs.
