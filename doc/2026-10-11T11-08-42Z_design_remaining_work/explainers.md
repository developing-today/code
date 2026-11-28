# Explainers: the terms behind the open questions

- Date: 2026-10-11 (completed after round 2 research)
- Purpose: plain-language definitions and tradeoffs for the terms the open questions use. Each section names the decision it supports and the task doc with the detail. Numbers are measured unless marked as an estimate or an extrapolation.
- Research behind this file: [round 2 results](./research-round2/README.md) and [round 1 spikes](../2026-10-11T03-44-33Z_design_groups_events_delegation/spikes/README.md).

## 1. ChainExt (the event header extension)

**What it is.** A p2panda header extension: a struct we define, signed together with each event's header. p2panda-core accepts any serializable type as an extension. The only extension p2panda-core 0.7.1 ships is `PruneFlag`, a boolean that switches off the backlink check. We do not use it.

**What it carries.**

| Field | Meaning |
| --- | --- |
| `kind` | What the event is: a group operation, a message, a tag, a document, an invite, and so on. Text. |
| `group` | The group the event belongs to, as an optional hash. A node can index and route by group without decrypting the payload. |
| `after` | The causal frontier: hashes of the latest events from any author that this author had seen when writing the event. At most 32. |
| domain tag | The constant `id.events.v1`, so our header type cannot be confused with another protocol's. |

**Why it is needed.** The p2panda header already gives a per-author chain: the author's key, a sequence number, a backlink to the author's previous event, the payload size and hash, and a signature. That orders one author's events. It does not say which events from other authors this event depends on. The membership rules need that, because a removal must be ordered against the concurrent events it did or did not see. `after` carries that. Because it sits inside the signed header, nobody can change the causal links or the group without breaking the signature.

**Cost.** Each `after` hash costs about 34 bytes on the wire (a 32-byte hash plus its CBOR header). A full frontier of 32 costs about 1.1 KB per event.

**Decision (accepted).** ChainExt on every event from the start. There is no `LogExt` in p2panda-core 0.7.1, so there is nothing to switch to. Two formats would mean two migration paths, and we can discard data, so one format from day one is simpler.

**Alternative.** One log per group instead of one per device. Fewer logs, but the per-author order would move into the body, and sync could no longer work per device.

Details: [task 02](./task-02-events-and-storage.md).

## 2. Waiting events and waiting caps

**Waiting event.** An event that has arrived but cannot be applied yet, because a dependency has not arrived. The dependency can be a backlink, an `after` hash, or an auth dependency for the membership layer. The node holds the event until the dependency arrives, then applies it.

**Why caps.** Anyone can send events whose dependencies never arrive. Without a bound, a malicious or broken device can fill memory with waiting events. A cap bounds the memory.

**The two caps (accepted, configurable per world).**
- A count cap: at most 256 waiting events per author.
- A time cap: a waiting event is dropped after 7 days.

When a cap is hit, the node drops the event and does not acknowledge it. The author's events come back later through sync, once the dependency exists.

**Tradeoff.** A cap that is too small drops legitimate events from a device that was offline for a long time. Those events are not lost, because sync fetches them again. The cost is bandwidth and delay. A cap that is too large lets a bad peer use memory.

Details: [task 02](./task-02-events-and-storage.md).

## 3. Heads, storms, merges, and the own-latest rule

**Heads.** The latest events that no other event has named yet. If Alice and Bob each post once without seeing each other's post, both posts are heads.

**Storm.** A burst in which many members post at once, or a partition heals and every backlog arrives together. Each new event names the heads its author has seen, so after a storm there can be as many heads as there are members. S10 measured 40 heads after a 40-member burst.

**Why it matters.** An event can name at most 32 heads in its `after` list. With more heads than that, the next event cannot name them all, and the causal order becomes incomplete.

**Merge.** An event that names many heads, up to 32, so that they become one causal point. In the encryption layer a merge is also a key update, because it needs a new epoch. In DataScheme an update costs n-1 messages, so a merge in a 40-member group costs 39 messages. A merge is not free, so the rule is: merge only when the heads would exceed the cap.

**The own-latest rule.** Every event must name its author's latest event. S10 found that an event which truncated its heads to 32 and dropped its own previous event broke delivery under adversarial orders, with HPKE errors. Naming the author's latest event keeps per-author order checkable.

**Decision (accepted).** Cap 32. Always name the author's latest event. Merge when the heads would exceed the cap.

**Measured (S10).** Cap 32 needs 81 merges in S10's storm example. Cap 16 needs 162. Cap 2 needs 3,159 merges and about 1,150 waiting messages for three storms.

Details: [task 05](./task-05-encryption-and-history.md) and the S10 results.

## 4. Epochs, welcomes, and history modes

**Epoch.** A period between two key rotations, with its own secret. A message is encrypted under the secret of the epoch it was sent in.

**What rotates the key.** In the stock crate: creating the group, a removal, and an explicit update. A plain join adds no epoch in the crate. Our join path (from_join) adds an immediate update for every join, because a joiner must not read messages sent before the join. So in our design every join is also an epoch.

**Welcome.** The message that brings a new member up to date. It carries the epoch secrets that member may read. Its size grows with the epochs it carries: 146.5 bytes per epoch with the stock encoding, 40.0 with serde_bytes (S12, round 2 scheme-serde).

**History modes (decided: `full` by default).**

| Mode | What the joiner can read | Welcome per join, K = 1,000 rotations | Experience |
| --- | --- | --- | --- |
| `full` (default) | Everything since the group began | 147 KB stock, 40 KB serde_bytes (measured) | A new member sees the whole history, as members expect |
| `recent:128` | The last 128 epochs | 19.3 KB stock, 5.6 KB serde_bytes (measured) | Older messages show as unavailable until requested |
| `recent:32` | The last 32 epochs | 5.3 KB stock, 1.7 KB serde_bytes (measured) | Smallest join. Older messages show as unavailable until requested |

**What 32 means in practice.** An epoch is a rotation, not a time. In our design every join, removal, and explicit update is one epoch. So in a group with frequent joins, 32 epochs can cover only the last few hours or days of membership activity. In a quiet group they can cover years. The window is the same number of events, so the experience depends on how busy the group is. That is why `recent:N` is set per world.

**Why not 128.** At 128, a join costs about 3.6 times what 32 costs (19.3 KB against 5.3 KB stock). The gain is a window four times longer. The right number depends on the world: 32 for quiet groups, 128 for churn-heavy ones. Both are presets.

**Why not only `full`.** It grows with every rotation. At 10,000 rotations, extrapolated, a welcome is about 1.5 MB stock or 0.4 MB with serde_bytes. Re-adds are the worst case: total re-add traffic grows quadratically, 19.5 MB at 1,000 rotations and about 1.9 GB at 10,000 (extrapolated). The owner decided `full` is the default. `recent:N` remains available for churn-heavy worlds.

**Frame size.** The world session in this codebase caps frames at 1 MiB (`world_session.rs:48`). If a welcome has to fit one frame, stock `full` stops fitting near 7,150 rotations (inferred from the sizes above). So the identity transport needs an explicit cap, and large welcomes must be chunked. Decision needed: see the owner questions in the index.

**Two rules that the round 2 results make mandatory.**
1. A `recent:N` joiner cannot read older messages through the crate. The adapter must check the group secret ID before it calls `receive`, and keep pre-welcome history outside the crate. In round 2 delivering that older history to the crate failed the welcome in 80 of 120 runs (agent-reported; the reproduction is the first red test in Phase 4).
2. Welcome bytes depend on hash-map order, so the welcome must be sorted before it is signed or hashed.

**Lazy full history (new item).** A joiner who chose `recent:N` can later ask a member for older epoch secrets. The member sends them encrypted to the joiner. This needs an online member and a rule for who may answer. It is a Phase 4 item.

Details: [task 05](./task-05-encryption-and-history.md) and [welcome-recent results](./research-round2/welcome-recent/RESULTS.md).

## 5. Range sync versus LogSync

Both sync two peers' event sets. They differ in how they find what the other side lacks.

**LogSync** (p2panda-sync). Each peer sends the latest position of every log (here, every device chain), and the other side replies with what is missing.
- Round trips to start: 2.
- Control bytes at 1,000 chains: about 76 KB, roughly 76 bytes per chain. Grows linearly with chains.

**Range sync** (ours). Each peer summarizes sorted ranges of (author, seq) keys with a fingerprint and a count. Mismatched ranges are split until a range is small enough to send its keys.
- Round trips to start: 7 to 8.
- Control bytes at 1,000 chains: about 6.5 KB on real stores (S7). Grows slowly with chains.
- Fingerprints are XORs of 16-byte BLAKE3 hashes, with counts.

**The crossover, measured in round 2 (range-config).** The earlier 40-chain figure held only for two new events. The real crossover is about 3.5% of chains, which is the proposed rule: use LogSync when the estimated difference exceeds 0.035 times the number of chains, or when bootstrapping. Otherwise use range.
- At 4% of chains, range costs 1.2 times LogSync's control bytes. At 20%, 4.7 times.
- Range takes 7 to 9 flights against 2.

**Tradeoffs.**

| | LogSync | Range |
| --- | --- | --- |
| Bytes when the difference is small and chains are many | Worse, grows with chains | Better |
| Bytes when the difference is large | Better | Worse |
| Round trips | 2 | 7 to 9 (8 round trips at 150 ms is 1.2 s before data moves) |
| Code we own | Upstream, but it pulls in SQLite (below) | Ours, covered by the S7 and round 2 tests |
| Failure modes | Upstream's | Ours. Round 2 found a fork that aborts its whole batch and loses 20 unrelated events in the harness. That is a required fix with a test |

**Dependency note.** p2panda-sync depends on p2panda-store with default features, and those include `sqlite`. Using LogSync from p2panda-sync therefore brings SQLite back into the build, which the redb decision is meant to avoid. Options: (a) fork p2panda-sync and change that one dependency line to `default-features = false` (we own the fork), (b) implement LogSync ourselves over our traits (the protocol is small), or (c) accept SQLite as an unused build dependency. Preferred: (a), because the owner accepts forks we own.

**Decision (accepted).** Range above the crossover, LogSync below it. Both sides must support both. The initiator chooses, and the peer answers whichever method is asked for.

**K and T (accepted).** These are local split policy and never go on the wire. Each message is a list of entries, and each entry has an explicit upper bound and a mode: skip, fingerprint, or ID list. A receiver needs nothing from the sender, so two peers with different K and T still converge. Round 2 checked all nine wire pairs on 1,000 chains: equal stores, zero duplicates, exact symmetric difference. Costs with mixed settings, for reference: K16/T16 against K4/T8 gives 72,062 control bytes in 9 flights; K4/T8 on both sides gives 56,761 bytes in 11 flights; K2/T4 on both gives 66,001 bytes in 18 flights. Validation: K at least 2, T at least 1. T=0 never terminates on a single mismatched key, so it is rejected.

**Versioning.** Changing K or T needs no version bump, only validation. Changing the fingerprint function, the bounds encoding, the key layout, the framing, the leaf ID width, the mode bytes, or the frame cap needs a new protocol ID, `/id-sync/2`. The ID carries only the wire version.

**Frame cap.** A single flight above about 524,000 IDs exceeds the 16 MiB frame limit that the range harness uses (round 2, verified in memory). So bootstrapping a large store fails in either direction until flights are chunked to a size budget.

Details: [task 04](./task-04-sync-blobs-transport.md) and [range-config results](./research-round2/range-config/RESULTS.md).

## 6. Compaction (tags)

**What it means.** Tag history is a log of adds and removes. Compaction replaces a long prefix of that log with a snapshot of the current tag state, plus the operations after the snapshot's causal cut. The log gets shorter and the state stays the same.

**Why it is not simple.** A removal only works on an add that the remover had seen. If a removed add is dropped from the log, a new device that later receives an old copy of that add brings it back, because nothing says it was removed. S8 showed this with a tested counterexample: dropping stable clear bodies without a snapshot brings back removed adds for a new device.

**What a snapshot needs.** The state of every live tag, plus enough causal information (the observed dots and clock) that a new device applies later operations correctly.

**Numbers.** Tag state stays flat under churn: 117 B to 121 B across 20,001 operations. The log grows by about 131 bytes per operation, so the log is the part that needs compaction.

**Decision (accepted).** Snapshot at a causal cut, then the tail. Trigger: when a tag's log grows past a size threshold.

Details: [task 06](./task-06-tags-documents-search.md) and the S8 results.

## 7. Freshness window (invite accept time)

**What it is.** An invite accept carries a timestamp, `ctime`. The window is how far from the invite's creation and from "now" that timestamp may be.

**The rule.** `created - W <= ctime <= min(now + W, expires)`, with W = 5 minutes by default (accepted).

**Why it is configurable.** A home server may hold an accept for a while before it can forward it, which the design allows. A slow network needs a larger W. A public world may want a smaller one. The window can be set per server, per world, or per inviter, and the most specific setting applies.

Details: [task 07](./task-07-invites-and-pull.md).

## 8. Addresses, "authorize before connecting", and the public IP

**What the round 2 test showed (iroh-allowlist).** An iroh 1.3.0 endpoint learns the remote EndpointId only after the TLS handshake. Before that, it sees only the UDP source and the ALPN, and the TLS verifier accepts any key (`tls/verifier.rs:125-141`). The only identity hook runs after the handshake (`connection.rs:358-363`). Our candidates are queued when the connection is registered, before that hook runs (`remote_state.rs:387-409`).

In the test, an unlisted client's `connect()` returned Ok in all 60 refused runs, and the refusal arrived about 19 ms later. With a second local address, the refused client received the server's candidates in 17 of 30 runs. After the first stream it had the full set in 29 of 30 runs.

**What "authorize before connecting" means.** It means an endpoint refuses peers by identity: only listed peers may complete a connection and use its streams. That is access control. It does not hide addresses. Address privacy needs something else: a relay-only transport, or a patched iroh that withholds candidates until the peer is authorized.

**Decision (accepted, with the wording changed).** N30 is documented as address exposure to any peer that can connect. Tickets may carry addresses when they are needed, for example for a LAN or a direct path, and need none otherwise. The allowlist is an after-handshake gate for inbound connections, and a `before_connect` gate for our outbound dials.

**The owner's setup.** The server runs on the n0 relay, and all users see the server's public IP. This is accepted. It is also inherent: QAD gives the connected peer our public candidate, so any direct path reveals the public IP.

Details: [task 08](./task-08-connectivity-extensions.md) and [iroh-allowlist results](./research-round2/iroh-allowlist/RESULTS.md).

## 9. Scheme versions (N10, N20)

**What a scheme version is.** The encryption crate has no version field on its messages. Our term "scheme version" names one fixed encoding of the crate's messages, and the version is recorded with the group, so that a receiver knows which decoder to use.

- **v0** is the stock layout: the crate's own encoding.
- **v1** is our proposal: the same messages, with `serde_bytes` on the two `Vec<u8>` fields. These are the two-party plaintext and the X3DH ciphertext.

Nothing has shipped, and `pkgs/id` does not depend on p2panda-encryption yet. So v1 can be the first released scheme, and no stock group needs migrating.

**The options, with measured joiner welcome size per epoch (round 2 scheme-serde).**

| Option | Bytes per epoch | Fork needed | Notes |
| --- | --- | --- | --- |
| Keep stock (v0) | 146.5 | No | Largest. No migration cost |
| `serde_bytes` on both fields (v1) | 40.00 | Yes, two lines in the fork we already carry for from_join | 3.66 times smaller. Follows the crate's own serde convention. Incompatible with stock in both directions (verified) |
| Transcoder on the outer field only | 76.9 | No | Reaches the outer layer only, not the inner one |
| Our own compact encoding | about 40 (floor) | No, but custom crypto | At most about 15% better than v1, with custom crypto to maintain |
| Compression (bzip2) | 78 KB at K=1,000 | No | Does nothing for v1. Keys are random |

**Why v1 is the best choice.** It is the only option that reaches the floor. It is two lines in a fork we carry anyway. It uses the crate's own convention, not a format we invent. Nothing needs migrating, since nothing has shipped. Its costs are that stock and v1 cannot decode each other, that updates gain only about 10%, and that we need a version tag and a determinism fix. If the owner refuses the fork, the transcoder is the fallback.

**Where the version lives.** The version is bound to the signed group policy, and checked copies sit in the event header and in the welcome envelope (5 to 12 bytes in total). A receiver checks the copy against the signed policy, so a relay cannot downgrade the encoding.

Details: [task 05](./task-05-encryption-and-history.md) and [scheme-serde results](./research-round2/scheme-serde/RESULTS.md).

## 10. Signed tickets (N24)

**What a ticket is today.** An iroh `EndpointTicket` is the string `endpoint` followed by base32 of a postcard payload: a variant tag, the EndpointId, and any addresses. It has no signature and no expiry. The parser ignores trailing bytes (round 2 confirmed this for 0 to 300 bytes).

**What signing entails.** The paste string must still be a standard ticket, so that every iroh client accepts it. The signature has to live somewhere else, or in a larger string that only our clients understand.

**Options (round 2 ticket-signing, tested through the iroh-tickets parser).**

| Option | What the user pastes | Validation | Trade-off |
| --- | --- | --- | --- |
| A: standard ticket only | 124 characters | None | Simplest. Nothing proves who issued it |
| B: our signed envelope | 361 characters | Our clients verify the signature | Standard clients cannot parse it |
| C: standard ticket plus a signed invite event | 124 characters | The event is checked when present | The signature lives in the event, not the paste |
| D: ticket with appended signature | 277 characters | Parser ignores the bytes | A standard re-share strips the signature |
| E: A plus C (recommended) | 124 characters | Automatic when the event is available | Regular tickets work, and signatures are optional |

**Recommendation: E.** The pasted string stays a standard ticket. The invite link carries `by=<inviter key>` as the trust anchor. The inviter's signed invite event is checked automatically when it is present, and the joiner checks it before sending the accept. The user does nothing special.

**One open question for the owner.** The invite event must be readable before joining. That conflicts with an authorized pull, where only members can read a group. When the event is unreadable, the joiner can either proceed unverified, with a clear label, or refuse. Round 2 also found that `check_target` does not trim pasted text, so a paste with surrounding whitespace must be handled and tested.

Details: [task 07](./task-07-invites-and-pull.md) and [ticket-signing results](./research-round2/ticket-signing/RESULTS.md).

## 11. Ed25519 versions (decision 19)

**Newest.** ed25519-dalek 3.0.0 is the newest stable release (2026-07-06). Its only pre-release, 3.0.0-rc.1, is older. The newest 2.x is 2.2.0 (2025-07-09).

**What changed from 2.x to 3.x.** Key derivation, signing, and both `verify` and `verify_strict` are unchanged. The dependencies moved: curve25519-dalek 4 to 5, rand_core 0.6 to 0.10, signature 2 to 3, sha2 0.10 to 0.11. The minimum Rust version rose from 1.81 to 1.85. The one break that reaches us is that `generate()` now takes rand_core 0.10's `CryptoRng`.

**Are they compatible?** Yes on the wire. Round 2 checked 25 of 25 cases: identical keys, signatures, and RFC 8032 test vectors; mutual verification passes; non-canonical S is rejected by both; small-order and decoding results match. The only visible difference is error text. In Rust they are different types, and Cargo can build both in one tree, with duplicated curve25519-dalek, sha2, signature, ed25519 and rand_core. Conversion must go through bytes, because there are no `From` impls between versions.

**Why not move everything to 2.x.** iroh 1.3.0 and six other pinned crates require 3.x, and p2panda-core 0.7.1 requires 2.2.0. So "one copy in the tree" cannot hold while those pins stand. Moving the tree to 2.x would mean leaving iroh 1.3.

**Recommendation.** Stay on 3.0.0 and keep a small conversion shim at the p2panda boundary. Also change the five plain `verify` call sites (`directory_auth.rs:349`, `artifact.rs:300`, `directory_log.rs:100`, `directory.rs:396`, and one more) to `verify_strict`, which rejects non-canonical signatures and small-order keys. Each change gets a regression test. Advisories RUSTSEC-2022-0093 and RUSTSEC-2024-0344 do not affect the versions we use.

Details: [ed25519 results](./research-round2/ed25519/RESULTS.md).

## 12. Storage: redb everywhere, Turso spiked, libsql replaced (N1, N2)

**Decision (accepted, confirmed by round 2).** redb for the event store. The Turso spike did not change the answer. It is slower in every configuration measured: its WITHOUT ROWID tables full-scan every key lookup, a 200,000-claim run stalled after about 21 minutes at 99% CPU, and on rowid tables Turso took 57.7 s where redb took 11.5 s and SQLite 23.5 s. Its lookups were 7 to 22 times slower than redb. Turso's file was smaller (157.9 MB plus a 29.6 MB WAL, against redb's 269.5 MB, or 134.9 MB compacted). Its multi-process WAL and concurrent writers are experimental.

**libsql.** The web identity uses one table with four statements and an AES-256-GCM encrypted URI, in 17 lines of `src/web/identity.rs`. The owner's instruction is that libsql will be replaced. Round 2 recommends moving the identity tables to redb as well. The values are encrypted with AES-GCM, which is already in our lock file. That is about 120 to 150 lines (estimated) against about 40 lines for Turso, but Turso's encryption is experimental.

Details: [turso results](./research-round2/turso/RESULTS.md).

## 13. Storage engine tests: portable crash testing and LazyFS (N13)

**Decision.** The CI gate is an in-process fault-injecting backend for redb. LazyFS is an optional Linux job.

**The in-process model.** redb 4.3.0 has a storage backend trait (`redb::StorageBackend`, `db.rs:74`). The fault backend holds writes until `sync_data`, applies a survival policy at a simulated power cut, and can tear the in-flight write. The 50-transaction workload has 687 storage calls, and every one is a crash point: 7,808 runs. Of those, 7,621 reopen to the acknowledged state, 167 to acknowledged plus one (the in-flight commit became durable), and 20 fail to open before the first commit. There were zero failures. A control with a lying `sync_data` produces 5,731 failures, so the oracle does detect lost durability. A release sweep takes about 40 seconds.

**LazyFS.** It works, but only inside a user namespace, and it is not portable. It has to be built from source with g++ (clang fails on three narrowing errors), and its build fetches dependencies at configure time. Outside the namespace, `allow_other` is refused. It runs as an optional Linux job for a few corpus points.

**The corpus.** A checked-in TSV, one crash point per line, with the expected class. Every failure found in a sweep is appended with its fix, and a replay test fails if a recorded verdict changes. The workload's call count and a hash of its call trace are recorded, so a changed workload fails loudly instead of testing the wrong points.

**Two design items for the store (from the sweep).**
1. A crash during the very first creation leaves a file that redb refuses to open ("magic number mismatch"). Nothing acknowledged is lost, but the store cannot start. Fix: create the file under a temporary name, make the first commit, then rename it into place, and fsync the directory.
2. Debug and release builds recover different valid prefixes at four boundary points. This is not yet explained, so the corpus accepts either prefix for those points. If it turns out to be a redb bug, the engine decision gets a second look.

Details: [crash-tests results](./research-round2/crash-tests/RESULTS.md).

## 14. p2panda-store and the traits (N3, N5)

**Decision (accepted).** Implement the traits we need over redb, with no fork now. Round 2 store-fit counted eight traits and 36 methods. LogSync needs only three methods of `LogStore` (`get_log_heights`, `get_log_size`, `log_entries`), plus `TopicStore::resolve` for topic-based sync. Not covered by the traits, and so ours: fork proof and freeze, waiting caps and expiry, name and value indexes, and the unique (author, seq). A redb implementation of the needed subset is about 650 to 900 lines plus tests, medium risk. A fork would add the same work and delete 3,526 lines of SQLite code, medium-high risk.

**Two findings.** The `groups` feature of p2panda-store implies SQLite, so auth state goes in our own table. And p2panda-sync pulls the SQLite default (section 5), so LogSync needs the dependency fixed or a fork.

Details: [store-fit results](./research-round2/store-fit/RESULTS.md).

## 15. Toolchain, MSRV, and lints (N14, N15)

- **Git rev (N14, accepted).** The p2panda source stays on the git rev `3e84cac`, which needs Rust 1.96.
- **Toolchain (N15, accepted).** The newest stable is 1.99.0. No 1.99.x patch exists yet, and 1.100.0 is not published. The locked rust-overlay revisions already contain 1.99.0, so `rust-toolchain.toml` changes and no lock update is needed. No dependency needs more than 1.99.0 (the highest among 834 crates is 1.94.0).
- **Problem found.** `pkgs/id/default.nix` runs `rustup default 1.89.0` in its build phase, so the Nix check ignores the pin. Phase 0 fixes this and adds a test that the two agree.
- **Clippy.** Rust 1.99 adds lints that our configuration enables: about 50 `assert!(x.is_empty())` sites and 2 `chunks_exact` sites. CI does not use `-D warnings`, so this does not block CI, but task 12 requires no new findings. Recorded as a TODO for the lints review (P8.8).
- **MSRV.** The declared `rust-version` should be 1.96, the real minimum for the p2panda rev.

Details: [toolchain results](./research-round2/toolchain/RESULTS.md) and [task 12](./task-12-dependencies-and-toolchain.md).

## 16. Relays later: Cloudflare Tunnel, QAD, and Workers (N19, N29, N32)

- **No Tunnel account now (owner).** The server uses the n0 relay, and users see its public IP.
- **Later.** A local server behind a Cloudflare Tunnel, and several VPS for testing. The owner already has a Workers paid plan.
- **Tunnel limits (round 2).** A Tunnel carries HTTP and WebSocket only. QAD is QUIC over UDP port 7842, so a Tunnel-only relay gives no public candidate and no direct path.
- **QAD ordering (verified in source).** iroh's address probe uses only the first five relays in the relay map (`MAX_RELAYS = 5`, `net_report.rs`). A sixth entry can silently lose QAD. The relay map must put QAD-capable relays first, and a test must catch a map where they are not.
- **Workers (round 2).** A Worker can serve the pkarr HTTP PUT and GET, and DNS over HTTPS, using Durable Object storage. It cannot be a DHT node or serve DNS over UDP. The free plan caps a Worker relay at about 347 endpoints at the default republish rate.
- **Order recommended.** n0 now. Then a Tunnel home relay with n0 in every map, tested on real networks. A QAD VPS if n0's QAD disappoints. A Worker pkarr relay last.

Details: [cloudflare-workers results](./research-round2/cloudflare-workers/RESULTS.md).

## 17. BeeKEM

Not adopted now. The reasons, the measured costs, and the conditions for reconsidering it are in the [BeeKEM assessment](./beekem-assessment.md).
