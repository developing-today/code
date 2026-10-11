# Events, groups, delegation, removal, and history

- Date: 2026-10-11 (UTC 03:44)
- Scope: `pkgs/id`. Directory, signed artifacts, envelopes, tags, world capabilities, and the encryption and sync layers they depend on.
- Status: Proposed. Nothing here is built unless a section says "exists". This document does not change code.
- Source: design discussion on 2026-10-10 and 2026-10-11. Where a choice belongs to the owner, it is marked as a decision.
- Supersedes in part: [signed artifacts](../2026-10-10T04-58-36Z_design_signed_artifacts/2026-10-10T04-58-36Z_design_signed_artifacts.md). See the note in that folder and §17.

## 1. Intent

`id` needs one model for everything that changes: friendships, groups, memberships, permissions, delegations, tags, and documents. The model should:

- record every change as a signed event in its author's chain, so history can be replayed and checked;
- order events across authors by causal links, not by server arrival or wall-clock time;
- compute state (who is in a group, who may do what, which tags are current) as a fold over events, so any past point can be reconstructed;
- make groups first-class objects with a signed policy, so friendship, channels, and private groups are one mechanism with different settings;
- make delegation a first-class authorization step, with an explicit rule for what a removal does to delegations;
- support history-visibility and key-rotation policies, with a scheme identifier that lets key agreement change later;
- reuse p2panda and iroh where they fit, and write only the parts they lack.

Out of scope for now: time (witnesses and clocks), point-in-time content encryption, a tree-based key-agreement implementation, changes to the live editing protocol (the ProseMirror collab server stays as it is), and release concerns.

## 2. Decisions

| Topic                 | Decision                                                                                                                                                                                      | Status                                                 |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| Ordering              | Each author has one signed chain. Cross-author order is a causal DAG built from `after` links.                                                                                                | Decided                                                |
| Forks                 | A second, different event at a chain position is refused. Both events are stored as signed proof, and the author is frozen from that position.                                                | Decided                                                |
| Signed encoding       | Deterministic CBOR with strict decoding. Replaces serde_json re-serialization. Crate choice open (§4.5).                                                                                      | Decided; crate open                                    |
| Hashes and signatures | BLAKE3 for event IDs and content. Ed25519 for signatures.                                                                                                                                     | Decided                                                |
| Events                | Every mutable change is a signed event: friend requests and answers, removals, group creation, policy, memberships, delegations, revocations, tags, untags, documents, device keys.           | Decided                                                |
| Groups                | A group carries a signed policy record. Kinds are templates over policy fields, not hard-coded types.                                                                                         | Decided                                                |
| Friendship            | A two-member group template: cap 2, mutual request and accept, dissolves when either side is removed.                                                                                         | Decided                                                |
| Removal               | Strong remove is the default, set per group. Weak is available per group. No per-removal override.                                                                                            | Decided                                                |
| Delegation            | First-class in the auth layer. The fate of a removed member's delegations is a policy choice: `keep_prior`, `cascade`, or `transfer`.                                                         | Decided in principle; default open (§7.3)              |
| Time                  | Deferred. Causal order only. Claimed time is display-only.                                                                                                                                    | Decided                                                |
| Large groups          | No hard size cap. Encryption is optional per group. Large channels can run without end-to-end encryption until a suitable scheme exists.                                                      | Proposed; decision open (§8.6)                         |
| Joiner history        | Per-group policy: `full`, `from_join`, or `causal` (causal deferred).                                                                                                                         | Decided as policy; default open                        |
| Key agreement         | Behind a trait. The scheme and version live in the signed group policy.                                                                                                                       | Decided; trait shape open                              |
| Libraries             | Adopt p2panda-auth and p2panda-encryption. Adopt p2panda-core if spike S1 passes. Pin git revisions of p2panda `main`. Use iroh-blobs and iroh-tickets. Spike p2panda-sync against iroh-docs. | Decided in direction; spikes pending                   |
| Tags                  | Tags become events. The α/Ω layout stays as a local read index. Existing `tags.rs` storage is discarded.                                                                                      | Decided                                                |
| Existing data         | No migration. Existing artifacts, directory logs, and tags may be discarded.                                                                                                                  | Decided                                                |
| Invitations           | Seitan-style invite keys authorize a join. Iroh tickets give reachability only.                                                                                                               | Decided in principle; Seitan details to verify (§10.4) |
| Publication           | The integrated branch was pushed to `origin/main` at `d32b6aaf` on 2026-10-11.                                                                                                                | Done                                                   |

## 3. Current state

What exists on `main` at `d32b6aaf`, and where.

| Area                 | Location                                | State                                                                                                                                                                                                                                                                                                                                                                                                |
| -------------------- | --------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Signed artifacts     | `pkgs/id/src/artifact.rs`               | Exists. One chain per author, with `prev` and `seq` (starts at 1). Kinds: `friend_remove`, `home_declared`, `document`, `tag`. Hash is SHA-256 over the serde_json encoding, and the signature is Ed25519 over the same JSON. `after` is checked for shape (at most `MAX_AFTER` = 32) but is not used for ordering or removal. Pull and push run on `/id-artifact-pull/1` and `/id-artifact-push/1`. |
| Envelopes            | `envelope_net.rs`, `envelope_outbox.rs` | Exists. Friend request and accept envelopes on `/id-envelope/1`. Targets are endpoint tickets or node IDs. Up to `MAX_ATTEMPTS` = 12 tries, backoff capped at 6 hours. Envelopes expire after `ENVELOPE_TTL_MS` (30 days). Clock skew window is 5 minutes.                                                                                                                                           |
| Directory            | `directory.rs`, `directory_view.rs`     | Exists. State is rebuilt by replaying a log of entries (`Directory::replay`). Friendship is derived state. At most `MAX_PENDING_FRIEND_REQUESTS` = 20 pending requests per sender per server. `FriendRemoved` deletes the friendship and the pending requests between the pair, and records the request IDs it voids.                                                                                |
| Roles                | `directory.rs` `Level`                  | Exists: Access, Read, Write, Manage, Moderator, Admin.                                                                                                                                                                                                                                                                                                                                               |
| World capabilities   | `world.rs`                              | Exists. Bearer `JoinCapability` tokens in a parent chain, with scopes and `CapabilityBounds { expires_at, uses }`. `effective_scopes` checks friendship live on each use. `delegate_to_friend` refuses non-friends. The hosting world enforces use counts, and they survive restore.                                                                                                                 |
| Petnames and invites | `petname.rs`, `invite.rs`, `qr.rs`      | Exists. Petnames are local and not replicated. Invite links carry an account key, a node, and a name. They set petnames and do not join anything.                                                                                                                                                                                                                                                    |
| Tags                 | `tags.rs`, `tuple.rs`                   | Exists. α/Ω key pairs on iroh-docs, in Global, Node (private by default), and Custom namespaces. AND search. Separate from artifacts.                                                                                                                                                                                                                                                                |
| Records replication  | `world_records.rs`                      | Exists. Read-only iroh-docs ticket, pulled once by `world mirror`. Not continuous.                                                                                                                                                                                                                                                                                                                   |
| Discovery            | `discovery.rs`, `serve.rs`              | Exists. distributed-topic-tracker with iroh-gossip. Announcements carry node IDs and no addresses.                                                                                                                                                                                                                                                                                                   |
| Transport            | `serve.rs`, `world_session.rs`          | Endpoint uses the N0 preset, so bare node IDs resolve through n0 DNS and pkarr. `MAX_FRAME_BYTES` = 1 MiB.                                                                                                                                                                                                                                                                                           |

Known gaps:

- Strong-remove filtering does not exist. `after` is stored but never acted on.
- Pulls are unauthenticated. Anyone who holds a key and can reach a home server can read that account's chain.
- Pages are capped at 100 items by count only. With 64 KiB documents, a full page is about 6.4 MB and cannot fit a 1 MiB frame. This is a bug.
- Fork proofs are not stored. Forks are refused and dropped.
- Friend removal voids only the request IDs it names.
- Tags and artifacts are two separate systems.

## 4. Event model

### 4.1 Event fields

An event is one signed item in one author's chain. Field names are conceptual. The encoding is CBOR (§4.5).

- `author`: the Ed25519 verifying key that signed. A device key or an account key.
- `seq`: position in the author's chain. Starts at 0 if we adopt p2panda-core's header, or at 1 as today. Decided in spike S1.
- `backlink`: the ID of the author's previous event. Absent at the first position.
- `after`: the frontier (§4.3). Required in every event. May be empty.
- `kind`: text, such as `group.create`, `group.policy`, `member.set`, `group.join`, `delegate`, `revoke`, `invite`, `device.add`, `device.revoke`, `tag.set`, `tag.clear`, `document.put`, `checkpoint`.
- `group`: optional group ID for group-scoped kinds.
- `body`: a kind-specific CBOR map.
- `payload_hash`, `payload_size`: BLAKE3 and size of the encoded body. The same shape as p2panda-core's header.
- `signature`: Ed25519 over the encoded header, without the signature field.
- `id`: BLAKE3 over the encoded header. Recomputed on receipt and never trusted from the sender. This replaces today's SHA-256 `hash`.

### 4.2 Chains and forks

- Each device has one key and one chain. An account is authorized through device delegation (§4.6).
- An event is accepted into a chain only when `seq` is the next expected value, `backlink` matches the held predecessor, and the signature verifies.
- A gap (a later `seq` arrives first) is held as waiting. Waiting events do not affect state. The receiver asks for the missing items.
- A fork is a second, different event at a position already held. Both are stored as signed proof. The chain is frozen at that position: events at or after it are held but not applied. Events before it stay valid.
- Recovery from a freeze: a group's managers remove the author (a membership event), or the author adds a new device key (§4.6).
- Losing a link does not roll anything back. The verified prefix stands, and later items wait.

Why refuse forks rather than keep both branches:

1. A position means "this author's nth statement". Two items at n make the history ambiguous, and readers could settle on different ones permanently.
2. Equivocation becomes attributable. Two signed items are a proof anyone can show.
3. One height summarizes a linear log, so sync stays cheap. Height-based diffs are blind to forks, so refusing them avoids silent gaps.
4. Authority checks need one history per author.

Costs: an accidental fork (a restored backup, one key on two devices, a crash between signing and saving) freezes that author at that point. Mitigations: persist before sending, use one key per device, and keep a documented recovery path.

### 4.3 Causal order (`after`)

- `after` is the frontier: the smallest set of event IDs, from any chains, whose causal closure covers everything the author had applied when writing. It is not a list of everything applied.
- Required in every event. An empty frontier means the author had applied nothing from another chain. Requiring the field removes the ambiguity between "unknown" and "none" that matters for strong remove.
- Capped at `MAX_AFTER` (32) per event. When the frontier needs more, the author publishes a `checkpoint` event that names up to 32 heads covering the rest. Later events name the checkpoint. Checkpoints are ordinary events.
- An event whose `backlink` or `after` names something not held waits. Its effect is not applied until its dependencies are held.
- The DAG is the union of chains plus `after` edges. It is acyclic, because an event can name only events its author had already seen.

### 4.4 State is a function of the event set

Key invariant: the state of a group, a friendship, or a tag set depends only on the set of applied events and their causal relations. It must not depend on arrival order or on when a node received an event.

- An event's validity is a function of its causal past and of the events concurrent with it.
- Some rules let a later-arriving event void an earlier-applied one (§6). A node may show an effect that a later event reverts. Views must say so.
- Deterministic tie-breaks use event IDs. Two nodes holding the same set compute the same result.
- Snapshots are caches keyed by the frontier they were computed from. They are never authoritative.

### 4.5 Encoding

- Deterministic CBOR. Decoders reject non-canonical encodings, so each signed header has exactly one byte form. This removes the serde_json re-serialization concern.
- p2panda-core encodes headers with the `cbor_core` crate and decodes strictly. Using the same crate would make interop with p2panda structures simpler. Confirm in spike S5.
- `version` sits inside the signed header. Unknown versions are rejected, not skipped.
- Fixed golden vectors for each kind, so an independent implementation can be checked against them.
- Binary payloads are CBOR byte strings. Base64 is not needed.

### 4.6 Devices and accounts

- Each device has its own Ed25519 key and its own chain. One key used on two devices forks, so this is required, not optional.
- `device.add` (signed by the account key) and `device.revoke` are events. A device's authority is a delegation from the account to the device key with an "act as account" scope (§7).
- Group events from a device are attributed to the account at their causal position.
- Removing an account removes its devices. The account's delegations follow the group's delegation policy (§7.3).
- Multi-device group encryption belongs to p2panda-spaces and comes with encryption (§8). The event model does not need it.

## 5. Groups

### 5.1 Policy record

A group has an ID: BLAKE3 of its `group.create` event, or the derived ID for friendships (§5.4). It holds members and a policy. The policy is set at creation and changed only by a `group.policy` event from a member whose role allows it at that causal position.

Policy fields:

- `member_cap`: optional maximum member count. Friendship uses 2. The resolver enforces the cap. When concurrent adds exceed it, the lowest event IDs up to the cap are kept, deterministically.
- `join`: `manager_add` (a manager adds members), `invite` (an invite event, §10), `mutual_request` (one side asks, the other accepts), or `open`.
- `on_member_removed`: `continue` or `dissolve`.
- `removal`: `strong` (default) or `weak` (§6).
- `delegation`: `keep_prior`, `cascade`, or `transfer` (§7.3). Default open.
- `history`: `full`, `from_join`, or `causal` (§8.3).
- `scheme`: key-agreement scheme and version (§8.7). `none` is allowed.
- `roles`: which roles may change membership, policy, and tags on the group's content.

### 5.2 Roles

Existing `Level` values plus p2panda-auth's levels:

- Pull: replicates ciphertext without keys. New.
- Access: sees that the group exists and its own membership. Existing.
- Read: sees members, levels, and the permission set, and reads content. Existing.
- Write: edits content and the description. Existing.
- Manage: adds and removes members at or below its own level, and may delegate. Existing.
- Moderator and Admin: keep their names. Proposed: Moderator is Manage plus server-scoped isolation settings. Admin is everything, including policy, name, and deletion. Whether these are group roles or server permissions is open (§18).

Only Admin may change policy.

### 5.3 Templates

- `friendship`: `member_cap` 2; `join` `mutual_request`; `on_member_removed` `dissolve`; `removal` `strong`; `delegation` `cascade`; `history` `full`. Each side has Manage over the other.
- `channel`: `join` `manager_add`, `invite`, or `open`. Many Read, some Write, few Manage. `history` `full`. `scheme` `none` until a large-group scheme exists (§8.6). No member cap.
- `private`: `join` `invite`. `history` `full`, or `from_join` if chosen. `scheme` is the DataScheme once encryption lands.
- `custom`: any combination of fields.

A template is a named preset written into the create event. The resolver works on fields, not kinds. A new kind needs new field values, not new resolver code.

### 5.4 Friendship

- Friendship ID: BLAKE3 over the label "friendship", the sorted account pair, and a counter. The counter starts at 0 and increments when a friendship dissolves. Two concurrent requests for one pair produce the same ID, so they collapse into one object.
- A request is a `group.create` with the friendship template. The friendship is active when both members have an applied event in the group. The other side's request or `group.join` completes it. A request in each direction therefore becomes one active friendship, instead of the current "already pending" refusal.
- Dissolve: either side's removal (or leave) dissolves the group. A mutual removal removes both, which gives the same result.
- Re-friending creates a new group with the counter incremented.
- `MAX_PENDING_FRIEND_REQUESTS` stays a receiver's local limit. It refuses requests and never changes state, so it does not need to converge.
- Expiry is not a state rule. Events do not expire. Delivery retries expire. A request that should lapse carries an explicit expiry in its signed body.

### 5.5 Creation and nesting

- `group.create` is an event. Members are listed at creation, and later additions are `member.set` events.
- Groups can be members of groups. This is how "everyone in G may do X" works (§7.1).

## 6. Removal

### 6.1 Terms

- Causal past of event E: every event E reaches through `backlink` and `after`, transitively.
- Concurrent: two events neither of which is in the other's causal past.
- Removal R of member M: a `member.set` or `leave` event that removes M.

### 6.2 Strong remove (default)

R voids:

1. Every membership, permission, and delegation event by M that is concurrent with R.
2. Every concurrent re-add of M.
3. Every event that depends on a voided event, transitively.
4. In a mutual removal cycle (A removes B while B removes A, concurrently), every member of the cycle.

R does not void:

- Events by M in R's causal past. The remover had seen them, so they stand. Delegations included, subject to §7.3.
- Content (documents and tags). See §6.5.

Rules 1 to 4 are the StrongRemove rules in p2panda-auth's resolver. Rule 4 is its mutual-cycle rule, and rule 3 is its dependency filter. p2panda-auth does not cascade causally prior adds. Our one addition is the delegation rule in §7.3.

### 6.3 Weak remove

Only the authority check applies. Events by M in R's causal future are invalid. Concurrent events stand.

### 6.4 No time window

The strong-remove rule has no timer. It is a property of the event set.

Example. Alice removes Joe at R. Joe, offline, had written X without seeing R. A week later Joe syncs and pushes X.

- X is concurrent with R. R is not in X's causal past, and X is not in R's.
- Under strong remove, X is voided.
- The server stores X as signed evidence. Views show "voided: concurrent with removal R".

Joe cannot escape this by naming R in X's `after`. Naming R means Joe had seen R. Then X is causally after R, and Joe lacks authority at that position. Either way X is void. An event by Joe stands only if R's author had already seen it.

Caveats:

- "Before" means what the remover had seen, not clock time. If the admin removed Joe without seeing Joe's latest add of Jim, that add is concurrent and void. That is correct, and the UI should give the reason.
- A node that has not yet received R may show X as applied. When R arrives, the node recomputes. Views need a state for this.
- Deciding voidness needs R and enough of the causal past to test concurrency. Retention and pruning must keep that (§18).

### 6.5 Content and removal

Documents and tags are not membership. Content concurrent with a removal follows a per-group content policy:

- `keep` (proposed default): stays visible, attributed to its author.
- `hide_concurrent`: hidden like voided membership.

### 6.6 Test cases to port

From p2panda-auth's resolver tests (names as found in its source): `mutual_removal_filter`, `mutual_remove_cycles_detected`, `mutual_remove_cycle_with_delegation`, `demote_remove_filter`, `remove_dependencies_filter`, `remove_readd_dependencies_filter`, `concurrent_readds_filtered`, `filter_only_concurrent_operations`, and `two_bubbles`. Each becomes a test here, plus the week-late case in §6.4.

## 7. Delegation

### 7.1 Operations

- `delegate{ grantor, grantee, scope, bounds, can_redelegate }`. The grantee is an account, a device, or a group.
- `revoke{ delegation }`. Ends one delegation from the revoke's causal position on.
- Scope is a set of permissions over a group or world. The mapping from current `WorldScopes` bits and group roles is fixed in the implementation phase.
- Bounds: `expires_at` (signed, converges) and `uses` (does not converge; §7.4).

### 7.2 Validity at a position

A delegation D by grantor G is valid at event E when:

1. G held the scope D grants, at D's causal position. Attenuation: D's scope is a subset of G's.
2. If G is not a root, G's own grant is valid at D's position under the same rule.
3. Roots are the owners or managers of the group at that position.
4. No `revoke` for D is in E's causal past.
5. D has not expired at E.

An action is valid when some chain from a root to the acting grantee is valid at the action's position, and every intermediate link has `can_redelegate`.

Attenuation follows keyhive's convergent capabilities: a grantor passes on only what it holds. Re-granting needs explicit permission.

Because validity is checked at the action's causal position, a changed chain fails later actions automatically. §7.3 decides what happens when a link's grantor is removed.

### 7.3 Removal fate of delegations

When M is removed (§6), each delegation M granted is handled by the group's `delegation` policy. The removal event may override the policy or name a successor.

- `keep_prior`: a delegation by M stays valid for actions after R, provided it was made before R. M cannot create delegations after R. Concurrent delegations by M are void under strong remove. A grant here is a fact about the past.
- `cascade`: a delegation by M is valid only while M still holds what it passes on. After R, actions that rely on M's delegations, and everything downstream of them, fail unless the grantee has an independent valid chain. A grant here depends on its grantor's standing.
- `transfer`: R names a successor S. The remover signs one event that re-issues M's outgoing delegations with S as grantor. Continuity becomes explicit and visible. S must hold the scope.

Under strong remove, concurrent delegations by M are void in all three modes. The mode decides only the delegations causally before R.

"Everything before now is fine" is `keep_prior` with R as the cutoff, where "before" means causally before.

Recommendation: `cascade` as the default. Removing someone should clean up what they handed out. Use `keep_prior` for groups where grants are settled facts. Use `transfer` when the remover names a successor.

Decision needed: confirm the default.

### 7.4 Bounds

- `expires_at`: signed in the delegation, so it converges. Checked at each action's position.
- `uses` (today's `CapabilityBounds.uses`): counts do not converge. Two holders on a partition can each spend the last use. Keep counts as local enforcement by the hosting node, as now. Never make them a group state rule. Whether counted grants should exist at all is open.

### 7.5 Migrating world capabilities

- Today: `world.rs` checks `are_friends` live in `effective_scopes`, and `delegate_to_friend` refuses non-friends.
- Target: a friendship is a group, and a world capability is a delegation inside that group. `effective_scopes` becomes a chain check at the action's position. The live friendship check goes away.
- Bearer join tokens stay bearer secrets on the join path. Each token maps to a delegation event carrying the token's digest. Revocation is a `revoke` event. The host checks the chain against its current frontier.
- The cross-world rules in [cross-world access](../2026-10-10T04-59-57Z_feature_cross_world_access/2026-10-10T04-59-57Z_feature_cross_world_access.md) and [world isolation](../2026-10-10T03-56-25Z_feature_world_isolation/2026-10-10T03-56-25Z_feature_world_isolation.md) overlap with delegation. Reconcile them when this lands. This document did not re-read them.

### 7.6 Why this is in the auth layer

p2panda-auth provides group membership, levels, the strong-remove resolver, and a `Conditions` trait. It does not provide delegation chains, cascade rules, or a rule for causally prior adds. Our auth layer adds `delegate`, `revoke`, attenuation and validity checks, and the three modes. Concurrency still comes from p2panda-auth's resolver, so the two rule sets do not diverge.

## 8. History, encryption, and key agreement

### 8.1 Terms

- Post-compromise security (PCS, sometimes called backward secrecy): after a rotation, a removed or compromised member cannot read content encrypted under the new key. Rotation provides this.
- Forward secrecy in the strict sense: compromising today's key does not reveal earlier content. Epoch keys do not provide this. Per-message ratchets do.
- Point-in-time access (keyhive's causal encryption): keys for a chosen causal frontier only. Deferred.

This document uses "post-compromise security" and "per-message forward secrecy" and avoids bare "forward secrecy".

### 8.2 Epoch keys (DataScheme)

From p2panda-encryption:

- Members publish key bundles: an identity key plus pre-keys.
- The group shares a random symmetric key per epoch. Content is encrypted with XChaCha20-Poly1305 under random 24-byte nonces. Each ciphertext carries an epoch hash.
- A membership change starts a new epoch with a new random secret. The secret reaches each remaining member through two-party rounds (2SM) encrypted with HPKE. Cost is O(n) messages per rotation.
- Members keep their past epoch keys. New members receive a key list.
- Group control messages are not encrypted, so membership is visible on the network.

### 8.3 Joiner history

The `history` policy decides what a welcome contains:

- `full`: every past epoch key. The joiner reads everything. This is p2panda's current behavior.
- `from_join`: only the current epoch and later. Earlier content stays unreadable to the joiner. Spike S3 must confirm DataScheme can welcome with the current epoch alone.
- `causal`: point-in-time keys, using per-item key wrapping (keyhive's model). Deferred.

Limits:

- A key cannot be unsent. A member who received a key keeps it.
- A later policy change affects future epochs, not keys already sent.
- Removed members keep every epoch they held. Hiding history from them means re-encrypting it.

### 8.4 Removal and rotation

- A removal starts a new epoch. The removed member cannot read the new epoch, and keeps the old ones.
- Messages sent before the rotation reached everyone still use the old key until the group catches up. Views should show each message's epoch.
- Rotating after a compromise is the same operation. It restores PCS.

### 8.5 Message encryption

A Double Ratchet per member pair gives per-message forward secrecy. Acknowledgements are group operations. State is O(n²) across the group, so it suits small groups where per-message secrecy matters. It does not suit channels. Deferred.

### 8.6 Scale and key agreement

Costs:

- DCGKA (2SM-based, DataScheme): O(n) messages per removal. Removing one member from a 5,000-member channel costs about 5,000 HPKE-encrypted messages. The welcome carries membership history, so it grows with churn as well as with size.
- Tree-based CGKA (TreeKEM, BeeKEM): O(log n) per update. Concurrent updates are resolved through the causal graph.

The figure of about 128 members is a performance observation for DCGKA. It is not a cap. No group should stop growing at a fixed size.

Proposal:

1. Key agreement is a trait, separate from membership. Membership comes from p2panda-auth and does not depend on the scheme.
2. The trait consumes causally ordered membership changes. It returns control messages, scheme-tagged welcome messages, and secret updates.
3. The trait exposes causal context to the scheme. BeeKEM uses it for concurrent updates. DCGKA assumes causal delivery and acknowledgements.
4. Channels default to `scheme: none`. The home server enforces read access, and there is no end-to-end encryption. Groups opt in to end-to-end encryption once a scheme fits them. This lets large groups work now. It also means the server holding the channel can read it, which must be stated plainly to members.
5. Private groups use DataScheme. A tree scheme (BeeKEM, ported or reimplemented) arrives later behind the same trait.

Decision needed: accept `scheme: none` as the channel default until a tree scheme is ready (§18).

### 8.7 Scheme identifier and change

The identifier is the cheap part. The following are required:

1. Signed policy. The scheme and version live in the group policy, which is changed by a `group.policy` event from an Admin. Per-message headers may repeat the scheme as a check, never as an input. Putting the scheme only in headers would let an attacker downgrade it.
2. Membership separate from key agreement, so swapping schemes does not rewrite membership history.
3. Version inside the signed bytes. Old readers reject unknown versions rather than misparsing them.
4. Negotiation. Key bundles advertise supported schemes. A change proceeds only when every current member supports the target scheme. Otherwise it is refused, or the non-supporting members are removed under the policy.
5. Permanent decoders. A scheme change starts a new epoch under the new scheme. Old epochs stay decryptable, so readers keep a decoder for every scheme that has ever produced an epoch.
6. Downgrade rejection. A message whose scheme differs from its epoch's scheme is rejected.

So the answer to "is an identifier enough?" is no. It is necessary, and items 1 to 6 are what make it useful later.

## 9. Tags, documents, and search

### 9.1 Tags as events

- `tag.set{ subject, name, value? }`. The subject is any event ID or document ID. Anyone may tag anything. Each claim is attributed to the signing key.
- `tag.clear{ claims }`. Names the claims it removes. Observed-remove: a clear removes only the claims its author had seen.
- A concurrent `tag.set` and `tag.clear` on the same claim leave the set in place (add wins), because the clear did not see it.
- Today's `del` (remove every value of a key) becomes a clear that names all observed claims with that subject and name.
- No author restriction on tags. A group may restrict tagging of its own content later through its policy.

The earlier rule that a tag's author must match its subject's author is withdrawn (§17).

### 9.2 Local index

- Keep the α/Ω layout. Each claim is indexed as `(subject, name, value)` and `(value, name, subject)`. Prefix scans answer "tags of X" and "subjects with value V".
- Keep tuple encoding (`tuple.rs`), AND search, and binary-safe values.
- The index is local and rebuildable from events. It is not replicated. The storage choice (p2panda-store on SQLite, or a plain store) is decided in spike S6.
- Existing `tags.rs` data and namespaces are discarded.

### 9.3 Documents and payloads

`document.put{ media_type, payload }`, where the payload is exactly one of:

- `json`: an inline CBOR value, limited to `MAX_DOCUMENT_BYTES` (64 KiB), as today.
- `bytes`: an inline CBOR byte string, under the same limit. CBOR carries binary natively, so the base64 overhead of the current JSON form does not apply.
- `blob`: a BLAKE3 hash and size. The bytes live in iroh-blobs, and the receiver verifies them while fetching. Used above the inline limit.

`media_type` is at most 128 bytes, as today. The event always stays small, because a large document is carried by hash.

Live editing: the ProseMirror collab server stays the live authority. A document event is produced from a snapshot. The trigger (for example, on idle close) is open (§18).

### 9.4 Search

- By subject: all claims on X, from anyone.
- By name and value: all subjects with `name=value` (for example `name=Seattle`).
- By author: all claims by a key or an account, from a per-author index.
- By kind: optional filter.
- Multi-term AND, as today. Trust filters (§9.5) apply after the index.

### 9.5 Versions, claims, and authority views

- A version is an event, usually a document. Its `after` names its parents. A merge names several. The heads are the versions that nothing supersedes.
- Claims are tags:
  - `name=Seattle` on a version;
  - `endorse` on a version;
  - `supersedes`, where the subject is the newer version and the value is the older one;
  - `canonical`, set by group managers, which points a name at a version.
- The viewer or server chooses a trust policy: a set of trusted keys or groups. Endorsement counts include only trusted endorsers, because throwaway accounts can inflate raw counts.

Answering "Seattle":

1. Find the versions carrying `name=Seattle`.
2. Reduce them to heads.
3. Rank the heads under the viewer's trust policy. Show the top head with the trusted parties who vouch for it.
4. Group the other heads by lineage, with the common ancestor and a diff.
5. Show proposals: versions whose `supersedes` points at the top head, with their endorsers.

Governance: `canonical` is set by a group's Admin, or by a quorum rule. Concurrent `canonical` events for one name are shown as contested. The view never picks one silently.

Viewpoints: "as this server", "as group G", "as me". Rewinding shows what was canonical at a given frontier.

Prior art: Matrix room upgrades (a tombstone points at the replacement), labeler services where clients choose which labelers to trust, Sigstore attestations, and git forks with pull requests. p2panda does not provide this layer. It is application work.

## 10. Invitations and joining

### 10.1 Seitan-style invite keys (authority)

As recalled from Keybase's Seitan V2 design (unverified; see §10.4):

- The admin makes a short key: 17 characters from a 30-character alphabet, about 83 bits. It is stretched with scrypt. HMAC derives an invite ID and an Ed25519 seed from the result.
- The admin posts an `invite` event containing the invite ID, an expiry, and the invite's public key. The public key can be published in the clear. Knowing it does not let anyone sign an accept.
- The invitee types the key out of band, derives the same ID and seed, and signs an accept.
- An admin checks that the invite is unused, unexpired, and unrevoked, and that the signature verifies against the public key. Then the admin posts `member.set`.

For us, the invite is a signed event in the admin's chain. The accept must match the invite's public key. A ticket confers nothing.

### 10.2 Iroh tickets (reachability)

- `EndpointTicket` from iroh-tickets 1.0.0, already a direct dependency, carries a node ID and addresses.
- A ticket grants nothing. It tells the invitee where to dial.
- A ticket can list several addresses, including the admin's declared home servers, so a stale address does not break the join.

### 10.3 Combined flow

1. The admin creates an invite with Seitan key K and shares a link: `id:join?group=G&invite=<public invite ID>&ticket=<EndpointTicket>`. K is typed in separately. Putting K in the URL fragment makes the link a bearer invite. That is a per-invite policy choice.
2. The invitee dials the ticket on a new join ALPN (`/id-join/1`, to be added) and presents the accept, signed with the derived key.
3. An admin who is reachable verifies the accept and posts `member.set`. If no admin is online, a home server can hold the accept. Any admin can process it later.

A home server can relay and hold an accept. It cannot add a member.

### 10.4 Unverified

- The Seitan details above are from memory. Two attempts to fetch the spec failed (403 at book.keybase.io, 404 at the keybase/client path). Verify against the spec before implementing. The exact KDF and HMAC labels matter.
- Do not implement Seitan from this document. Implement from the verified spec.

### 10.5 Current invite links

`invite.rs` invites set petnames for an account. They do not join anything, and they stay as they are. They can later carry a ticket for convenience.

## 11. Transport and sync

### 11.1 Current

- Envelopes: `/id-envelope/1`, one request per envelope, with a reply.
- Artifacts: `/id-artifact-pull/1` and `/id-artifact-push/1`.
- Records: a read-only iroh-docs ticket, pulled once by `world mirror`.
- Discovery: distributed-topic-tracker with iroh-gossip. Announcements carry node IDs and no addresses, so addresses are not published to the public topic.
- Bare node IDs resolve through the N0 preset (n0 DNS and pkarr, which are third-party services). Tickets avoid that for known peers.

### 11.2 Sync options

From the p2panda-sync source and README, and the iroh-docs README:

|            | p2panda-sync (LogSync)                                                                            | iroh-docs                                                                |
| ---------- | ------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| Data model | Append-only logs per author and log ID, grouped by topic                                          | Entries of (namespace, author, key) mapping to hash, size, and timestamp |
| Algorithm  | Exchange log heights, compute ranges, stream headers and bodies                                   | Range-based set reconciliation by fingerprints                           |
| Cost       | Grows with the number of logs                                                                     | Grows with the size of the difference                                    |
| Content    | Headers and bodies inline in the stream                                                           | Entries only. Content comes from iroh-blobs                              |
| Forks      | Blind to them: equal heights exchange nothing                                                     | Same author and key resolves by timestamp. No chain                      |
| Validation | Header, signature, and backlink on ingest                                                         | Entry signature only                                                     |
| Access     | Session hooks and an authoriser in p2panda-net (unreleased). Spaces tie connections to membership | Possession of a namespace ticket. No per-key or per-role rule            |
| Transport  | A trait over a sink and stream pair                                                               | Tied to iroh                                                             |

Interop: none at the sync layer, because the data models differ. They meet at blobs. A p2panda body hash is a BLAKE3 hash, which is also the iroh-blobs ID for the same bytes.

Plan: spike p2panda-sync's LogSync over our iroh endpoint for events (S1). Keep iroh-docs for the existing records feature. Decide after the spike whether iroh-docs also carries events. Either way, validation of roles is ours, because a namespace's write capability does not restrict keys or roles.

### 11.3 Blobs

- iroh-blobs: BLAKE3 verified streaming, range requests, hash sequences, and memory and filesystem stores. Its README warns that this version is not production quality. The owner has accepted that.
- p2panda-blobs 0.7.1 is a placeholder: `lib.rs` is a TODO and the crate has no dependencies. Not adopted.
- Decision: iroh-blobs for payloads. Spike S4 checks that a p2panda body hash fetches and verifies from iroh-blobs.

### 11.4 Limits

- Frame: `MAX_FRAME_BYTES` = 1 MiB.
- Pages: `PAGE_LIMIT` = 100 by count is the bug in §3. Fix: size pages by bytes, with a target of about 256 KiB. A page always carries at least one item. An item larger than the target is sent alone.
- Inline payloads: at most `MAX_DOCUMENT_BYTES` = 64 KiB. Larger payloads are blobs.
- Event size is limited per kind. Limits live in one constant set, checked against frame and page sizes.
- Raising a limit later: old readers refuse oversized items, and the chain stalls behind any refused item. Roll out in order: raise the accept limit on every node first, then let writers use it.

### 11.5 Discovery

- Keep distributed-topic-tracker with gossip. Announcements stay ID-only.
- Addresses come from tickets and from home-server declarations.
- p2panda-discovery's private set intersection is a later option, not part of this design.

## 12. Time

Deferred. Causal order is the only order used for validity.

Two ideas on record:

- Witness server. A server signs the hash of the current time together with a batch of event hashes received since its last witness. Each event in the batch is proven to exist at or after that time. Trust rests on the server, so multiple witnesses or public anchoring come later.
- Hybrid logical clocks, which p2panda-core's timestamp module supports, for display ordering.

Claimed time in events is display-only until the time design lands. It is not used for validity. The 5-minute skew window and the 30-day envelope expiry become delivery policy, not state rules.

## 13. Libraries

### 13.1 Adopt or evaluate

| Library                      | Use                                                                  | Decision                                                                                   |
| ---------------------------- | -------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| p2panda-auth                 | Group membership, levels, strong-remove resolver, `Conditions` trait | Adopt. Git dependency pinned to a reviewed `main` revision. Extended with delegation (§7). |
| p2panda-core                 | Header and operation format, CBOR, BLAKE3, extension trait           | Adopt if spike S1 passes. Otherwise borrow the shapes.                                     |
| p2panda-encryption           | DataScheme (DCGKA over 2SM). Later MessageScheme and spaces          | Adopt behind our key-agreement trait (Phase 5).                                            |
| p2panda-sync                 | LogSync for event transfer                                           | Spike S1 against iroh-docs.                                                                |
| p2panda-stream               | Causal ordering, out-of-order buffer, validation pipeline            | Spike. May replace our waiting logic.                                                      |
| p2panda-store                | SQLite store with transactions                                       | Spike S6 for the local index.                                                              |
| p2panda-net                  | Own endpoint, discovery, gossip, sync managers                       | Not now. Revisit for SyncHook and the authoriser.                                          |
| p2panda-discovery            | Topic discovery by private intersection                              | Not now.                                                                                   |
| p2panda-blobs                | Blob storage                                                         | Not adopted. Placeholder.                                                                  |
| p2panda (node API)           | High-level API                                                       | Not adopted. Wrap the lower crates.                                                        |
| p2panda-spaces               | Multi-device group encryption                                        | Later, with encryption.                                                                    |
| iroh-blobs                   | Payloads                                                             | Adopt. Already in the lock file.                                                           |
| iroh-tickets                 | Invites and addresses                                                | Adopt. Direct dependency.                                                                  |
| iroh-docs                    | Records replication, and possibly sync                               | Keep for records. Decide after S1.                                                         |
| keyhive crates               | Convergent capabilities, causal encryption, BeeKEM, threat model     | Design input. BeeKEM may be ported behind the trait later. Not adopted as crates.          |
| Subduction and Sedimentree   | DAG-aware sync, relay storage of ciphertext                          | Design input. Revisit if DAG sync becomes the bottleneck.                                  |
| iroh-willow and Meadowcap    | Scoped write delegation, confidential sync                           | Not adopted. No revocation or rewind model.                                                |
| localfirst/auth (TypeScript) | Signature chains, roles, Seitan-style invites                        | Reference only.                                                                            |

### 13.2 Why p2panda over keyhive

- It is generic over bytes and data types, and it integrates log, sync, store, and groups.
- Its iroh versions match ours (iroh 1.3.0 and iroh-gossip 0.101.0 in our lock file).
- It is dual licensed, MIT or Apache-2.0.
- Keyhive has what p2panda lacks: delegation with cascading revocation, causal encryption, BeeKEM, and a written threat model. Those become design input for §7 and §8. We do not depend on its crates.

### 13.3 Dependency reporting

- Each new Cargo dependency is reported with its name, version, reason, and the feature it enables, as `pkgs/id/AGENTS.md` requires.
- Correction: AGENTS.md's `nix-common.nix` rule applies to dev tools in the Nix shell. Cargo crates go in `Cargo.toml`.
- Expected additions: p2panda-core, p2panda-auth, and p2panda-encryption as pinned git dependencies; a CBOR crate (`cbor_core` if S5 confirms it, to match p2panda-core); p2panda-sync and p2panda-store only if the spikes choose them.
- Already present through other crates: blake3, ed25519-dalek, x25519-dalek, chacha20poly1305, and hpke.

## 14. Views

- CLI: `id tag set` and `del` keep their shapes and publish events. `list` and `search` read the local index. Output shows the author and event ID, so provenance is visible.
- New verbs, proposed: `group create|policy|join|add|remove|role|list`, `delegate`, `revoke`, `publish document|tag`, `events`, `frontier`, `history`.
- Explorer: new lines go in the shared grammar in `directory_view.rs`, so the SSH and HTML explorers stay in step.
- Web: push updates over the websocket when events are applied or voided. Sections for waiting events (with the missing dependency), voided events (with the removal that voided them), and fork proofs. A verification mark on each event.
- "As of" means a frontier, not a clock time (§12).
- Every rejection gives a reason: "voided: concurrent with removal R", "waiting: missing X", "frozen: fork at seq n".

## 15. Phased plan

### Phase 0: immediate, independent of this design

- Size pages by bytes (§11.4). This is a bug in the current code.
- Triage the failing and hanging tests in §16.
- Decide whether pulls require authentication (§18).

### Phase 1: spikes

Nothing here merges as product code. Each spike ends with a recorded decision in this document.

- S1 Event transport. p2panda-core's header with our extension fields, moved by p2panda-sync's LogSync over our iroh endpoint. Exit: header adopted or borrowed; sync path chosen; sequence base (0 or 1) chosen.
- S2 Auth. A p2panda-auth group with the friendship template and the removal rules, run against the week-late, transitive-void, and mutual-cycle cases. Exit: `after` mapped to p2panda-auth's dependencies, or a shim.
- S3 Joiner history. A DataScheme welcome that carries only the current epoch. Exit: `from_join` feasible or not.
- S4 Blobs. A p2panda body hash fetched and verified from iroh-blobs. Exit: confirmed.
- S5 Encoding. Deterministic CBOR with golden vectors, rejecting non-canonical input. Exit: crate chosen.
- S6 Index. Local index on p2panda-store (SQLite) or a plain store. Exit: chosen.

### Phase 2: events and chains

- Event format, chains, fork proof and freeze, waiting for gaps, mandatory `after`.
- Move `friend_remove` and `home_declared` to events. Directory state is derived from events, and the entry log replay is replaced.
- Exit: convergence tests (shuffled and duplicated delivery gives identical state), fork test, gap test, page-size test.

### Phase 3: groups and friendship

- Policy records, templates, friendship as a template with dissolve, strong and weak removal, provisional view states.
- Explorer lines and web sections.
- Exit: friendship tests (both sides, concurrent requests, dissolve on either side, re-friend), and the week-late removal test.

### Phase 4: delegation

- `delegate`, `revoke`, attenuation, validity at position, the three modes, `transfer`, `expires_at`.
- World capabilities move to chain checks. The live friendship check is removed. Bearer tokens map to delegations.
- Exit: mode tests for each, and the existing world capability tests still pass.

### Phase 5: tags and documents

- Tag and clear events, the local index, removal of `tags.rs` data, documents with inline or blob payloads, search.
- Exit: observed-remove tests, search tests, blob fetch test.

### Phase 6: encryption

- Key-agreement trait. DataScheme for private groups. History policy. Rotation on removal. Scheme negotiation and downgrade rejection.
- Exit: a removed member cannot read the new epoch; the joiner's history policy holds; a downgrade is rejected.

### Phase 7: scale and authority

- Channel template with `scheme: none` by default. A tree scheme when needed. Checkpoints for large frontiers. Trust-ranked views.
- Exit: a 5,000-member channel test without end-to-end encryption; a frontier checkpoint test.

Deferred: causal encryption, time witnesses, message encryption, and multi-device spaces (which may land with Phase 6).

## 16. Tests

- Resolver conformance: port the p2panda-auth cases in §6.6.
- Convergence: property tests. Random topological orders and duplicate deliveries of one event set must give one state.
- Week-late push: the example in §6.4.
- Forks: two events at one position store the proof, freeze the author, and leave earlier events standing.
- Gaps: waiting events do not change state, and apply once the gap fills.
- Delegation: attenuation violations are refused. Each removal mode behaves as §7.3 says. `transfer` requires holding the scope. Expiry is enforced.
- Friendship: concurrent requests collapse into one. Accept after request. Dissolve on either side. Re-friend increments the counter.
- Tags: observed-remove. A concurrent set and clear leave the set standing.
- Encoding: golden vectors. Non-canonical CBOR is rejected. Unknown versions are rejected.
- Downgrade: a message whose scheme differs from its epoch's is rejected.
- Pages and frames: a page never exceeds the frame cap. An oversized single item is sent alone.
- Two-endpoint iroh integration tests, following the pattern in the artifact tests.
- Existing failures in this environment: `world_compile` tests need `host.wasm`, which is absent. `cli_integration::test_world_module_shared_over_iroh_and_web_presentations` hangs, and `test_worlds_compile_roc_sources_on_the_fly` fails. Both also occur on the pre-merge baseline `3546fd79`, so neither comes from the integration work. They must be fixed or quarantined before Phase 2 exits.

## 17. Corrections to earlier docs and discussion

- The signed-artifacts design said a tag's author must match its subject's author. Withdrawn. Anyone may tag anything.
- The current code signs serde_json re-serialization. Replaced by deterministic CBOR.
- Base64 for binary payloads is unnecessary with CBOR byte strings.
- The figure of about 128 members is a DCGKA performance observation, not a cap.
- p2panda has no causal encryption. Keyhive does.
- `after` is a frontier. It is required in every event and is not a list of everything applied.
- Removal is state, not a tag.
- The two-edge mutual-add friendship is replaced by the friendship group template.
- "p2panda-core is fork-tolerant and stores both branches": the types do not wedge, but what its store does with a second branch is not verified. Verify in S1.
- Seitan details are from memory (§10.4).
- Cargo crates go in `Cargo.toml` and are reported. The `nix-common.nix` rule covers Nix dev tools (§13.3).
- Envelope expiry is a delivery policy, not a state rule. Events do not expire.

## 18. Open questions

Decisions needed from the owner:

1. Default delegation mode on removal: `cascade` (proposed) or `keep_prior`.
2. Channel encryption default: `scheme: none` until a tree scheme exists (proposed), or wait for one.
3. Joiner history default for private groups: `full` or `from_join`.
4. Content on removal: `keep` (proposed) or `hide_concurrent`.
5. Strong versus weak removal is per group only (proposed). No per-removal override. Delegation modes can be named per removal.
6. Moderator and Admin: group roles, or server-scoped permissions.
7. Use-count bounds: keep as local enforcement, or drop.
8. Frontier cap: 32 with checkpoint events (proposed), or a larger cap.
9. Fork recovery: removal by managers (proposed), or freeze only.
10. Local index storage (S6).
11. Event sync: p2panda-sync or iroh-docs (S1).
12. Pinning p2panda: git revisions of `main`, each one reviewed before it is bumped.
13. Pull authentication: require a signed read, or accept that anyone holding the key can read the chain.

Technical questions:

- Retention and pruning: what must survive for concurrency checks (§6.4).
- Snapshot trigger for live editing documents.
- Timing of multi-device work relative to encryption.
- Verify Seitan against the spec.
- Migrate existing world capability tokens without breaking bearer joins.
- Reconcile cross-world access and isolation with delegation.
- Whether the scheme identifier, with items 1 to 6 in §8.7, is enough for later changes. §8.7 says the identifier is necessary but not enough, and the rest is required.

## 19. Glossary

- Event: a signed item in one author's chain.
- Chain: one author's events in sequence. Each links to the one before it.
- Backlink: the ID of the previous event in the chain.
- Frontier (`after`): the smallest set of events an author had applied that covers everything else it had seen.
- Causal past: the events an event reaches transitively through `backlink` and `after`.
- Concurrent: neither event is in the other's causal past.
- Fork: two different events at one chain position.
- Waiting: held but not applied, because a dependency is missing.
- Voided: applied earlier, now without effect, because a later event made it void (for example, strong removal).
- Policy: a group's signed rules.
- Template: a named policy preset.
- Delegation: a signed grant of scope from grantor to grantee, with attenuation.
- Cascade, keep_prior, transfer: the three removal modes for delegations.
- Epoch: a period during which one group secret is in use.
- Welcome: the message that gives a joiner its keys.
- PCS: post-compromise security.
- Scheme: a key-agreement method with a version.

## 20. Discussion record

- Friend requests are one-way, and a friendship is active once both sides have an event in the group (§5.4).
- Friendship alone does not grant actions. Delegation does. Friendship gates delegation today (§7.5).
- Events for all mutable state, not only removals (§4).
- Arbitrary group kinds through policy records (§5).
- Concurrent removals last without a time limit (§6.4).
- Point-in-time reads are not needed now. The joiner history policy covers the near-term need (§8.3).
- The owner does not want a rewrite to block active users. This drives the channel default in §8.6 and the no-cap rule in §2.

## 21. References

Read for this document:

- p2panda READMEs: p2panda-core, p2panda-auth, p2panda-encryption, p2panda-sync, p2panda-blobs, p2panda-net, p2panda-discovery, p2panda-store, p2panda-stream, p2panda-spaces, p2panda.
- p2panda sources: p2panda-core `cbor.rs`, header, `logs.rs`, `traits.rs`, and the prune extension. p2panda-auth `group/resolver.rs` (StrongRemove and its tests), `graph.rs`, `validation.rs`, and `access.rs` in part. p2panda-encryption `data_scheme/dcgka.rs` in part, `traits/dgm.rs`, `traits/ordering.rs`, and the key manager and key registry traits. p2panda-sync `protocols/log_sync.rs` in part. p2panda-blobs `Cargo.toml` and `lib.rs`.
- p2panda posts: "Local-First group- and message encryption in p2panda" (2025-02-24) and "Access Control in Decentralised Systems" (2025-07-28).
- Keyhive: README, `design/README.md`, the causal encryption and group membership designs, the threat model in part, and the BeeKEM overview.
- Subduction README, and the Sedimentree core README and design pages.
- iroh: iroh-docs README, iroh-blobs README, iroh-tickets 1.0.0 source (`endpoint.rs`), and iroh 1.3.0 presets (`N0`, `N0DisableRelay`).
- iroh-willow README, and the Willow and Meadowcap specification pages.
- localfirst/auth README.
- Not retrieved: Keybase Seitan V2. Two fetches failed (§10.4).

Local code and docs:

- `pkgs/id/src/artifact.rs`, `directory.rs`, `directory_view.rs`, `envelope_net.rs`, `envelope_outbox.rs`, `world.rs`, `tags.rs`, `world_records.rs`, `discovery.rs`, `world_session.rs`.
- [directory sync](../2026-10-10T02-39-03Z_design_directory_sync/2026-10-10T02-39-03Z_design_directory_sync.md), [world isolation](../2026-10-10T03-56-25Z_feature_world_isolation/2026-10-10T03-56-25Z_feature_world_isolation.md), [signed artifacts](../2026-10-10T04-58-36Z_design_signed_artifacts/2026-10-10T04-58-36Z_design_signed_artifacts.md), [cross-world access](../2026-10-10T04-59-57Z_feature_cross_world_access/2026-10-10T04-59-57Z_feature_cross_world_access.md), [petnames and invites](../2026-10-10T07-00-00Z_feature_petnames_invites/2026-10-10T07-00-00Z_feature_petnames_invites.md).
- Research note outside `main`: `/home/user/work/docs-artifacts/doc/2026-10-10T06-00-00Z_research_identity_capabilities/`.

Related, not re-read for this document: `2026-03-20T05-30-00Z_feature_collaborative_cursor_enhancements`, `2026-03-21T05-47-48Z_feature_content_modes`, `2026-03-22T00-00-00Z_feature_peer-discovery`.
