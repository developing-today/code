# Directory sync: sessions, friend envelopes, and replication

Related: [accounts, groups, contacts and delegation](../2026-10-09T12-00-00Z_feature_directory_accounts_groups/2026-10-09T12-00-00Z_feature_directory_accounts_groups.md)
(Phase 12 limitations), and
[attenuated capabilities](../2026-10-09T09-59-07Z_design_attenuated_capabilities/2026-10-09T09-59-07Z_design_attenuated_capabilities.md).

Status: design only. Nothing here is implemented. Findings were read from the
code and not run. Replay cases should get a failing test before any fix.

## Intent

Phase 12 lists three gaps that block a multi-server directory:

1. Sessions, pending sign-in codes and replay nonces are in memory only. A
   restart signs everyone out and drops pending codes.
2. Friend envelopes are verified by `receive_request` and `receive_accept`, but
   nothing delivers them. They are copied by hand.
3. Two servers can each accept friendships, change membership and delete
   groups. Nothing merges their changes.

This document sets out options for each, recommends one, and lists what the
user must decide. It does not choose for the user where the answer depends on
how the servers are meant to be used.

## Current state (from the code)

- **Directory journal.** Each world has `directory.jsonl` next to its
  `journal.jsonl` (`src/world_store.rs:325`). `Directory::open` replays it
  (`src/directory.rs:395-424`). `replay` fails on the first refused line.
  There is no compaction. `Directory::append` fsyncs one line per change
  (`directory.rs:431`).
- **The apply path.** Every change is a `DirectoryEntry`. `apply`
  (`directory.rs:1124`) clones the directory, runs `change`, runs
  `ensure_admins`, appends to the journal, and only then commits. Replay runs
  the same `change` code.
- **Sessions and codes.** Both live in `DirectoryAuth`
  (`src/directory_auth.rs`), held by the world core (`src/world.rs`, field
  `auth`). Sessions last `SESSION_TTL_MS` (12 hours) and are keyed by the
  SHA-256 of the token. Codes are six digits, valid 15 minutes, five attempts,
  and one resend a minute.
- **Replay guard.** `ReplayGuard` is held by `WorldHub` (`src/world_hub.rs:172`,
  field `replays`), so it is per process and covers every world. It remembers
  `key:nonce` for `SIGNED_WINDOW_MS` (5 minutes) and holds at most 4096 entries.
- **Envelopes.** `Envelope` (`directory.rs:285`) has `kind`, `from`, `to`, `at`
  and a signature over `{kind, from, to, at}`. `verify` (`directory.rs:325`)
  checks the signature only. It does not check `at` against a clock, has no
  nonce, no audience, and no expiry. `receive_request` (`directory.rs:1018`)
  and `receive_accept` (`directory.rs:1042`) have no callers outside tests.
- **Friend removal** is journaled on the server that does it
  (`remove_friend`, `directory.rs:1065`). No envelope tells the other server,
  so the two sides already diverge after a removal.
- **Explorer.** The grammar (`directory_view.rs`, `parse_line`) has
  `request_friend` and `accept_friend`, but no `receive`. No contact card code
  exists, though Decision 8 of Phase 12 describes one.
- **Group IDs** are sequential. `change` requires
  `*id == self.next_group` for `GroupCreated` (`directory.rs`, the
  `GroupCreated` arm, around line 1217).
- **Uniqueness** is checked at apply time. A key may belong to one account
  (`AccountKeyAdded`, around line 1164), and an email may belong to one account
  (`AccountEmailConfirmed`, around line 1188).

---

## 1. Session persistence

### The problem

Sessions and codes are volatile. A restart forces a sign-in again and loses
pending codes. The loss is small for sessions. The replay guard is the real
concern, because it is also volatile. A signed request captured before a
restart is accepted again after it, until its 5-minute window closes.

### What must not be persisted

- **Codes.** A code digest is SHA-256 over `id-code-v1`, the address and the
  six-digit code. The address is known to anyone who reads the file, so the
  whole code space is about one million hashes. A persisted digest would let
  anyone with the file recover a pending code. Codes stay volatile in every
  option.
- **Admin sessions.** `Principal::Admin` is a session opened by the configured
  admin token. Admin sessions stay volatile in every option.
- **Secrets.** Per the Phase 2 rule (test
  `the_journal_never_holds_a_credential_secret`), the file may hold only account
  IDs, token digests and expiry times. Digests of 256-bit random tokens are not
  usable as bearer tokens, because the lookup hashes the presented token.

### Options

1. **Journal hashed account sessions in `directory.jsonl`.** Add
   `SessionOpened { account, digest, expires_at }` and `SessionClosed { digest }`
   as `DirectoryEntry` variants.
   - Fits the current pattern: one file, fsync, replay on open.
   - Expiry: entries whose `expires_at` has passed are skipped on replay. The
     file is never compacted, so every sign-in adds a line forever. The
     directory journal has no compaction today.
   - It mixes bearer state into the state that section 3 would replicate. A
     replicated log would then copy session digests to other servers.

2. **Separate local store, `sessions.jsonl`, in the world folder.** Append
   `SessionOpened` and `SessionClosed` lines, with the same fsync as the journal.
   At load, drop expired records and rewrite the file if many have expired.
   - Keeps the directory journal free of session churn, so the replication
     input stays small.
   - Adds a second file to keep consistent with the first. A crash between a
     session open and its response means the browser holds a session the
     server lost. The cost is a sign-in.
   - Covers account sessions only. Admin sessions and codes stay volatile.

3. **Keep volatile, and add a boot quiet period for signed requests.** Nothing
   is persisted. Signed requests are refused for `2 × SIGNED_WINDOW_MS` (10
   minutes) after the process starts.
   - Restart cost: users sign in again, and pending codes are resent after 60
     seconds.
   - Why 2 × window: a request captured before the restart has `at` at most
     the restart time plus the client's clock skew, which is bounded by the
     5-minute window. It is accepted only while `now - at <= SIGNED_WINDOW_MS`.
     After 2 × window, every pre-restart request is outside that bound, so the
     empty guard cannot accept it. A quiet period of exactly one window is not
     enough, because a client whose clock runs 4 minutes fast can still replay
     a request.
   - Cost: 10 minutes of refused signed requests after each restart. Session
     and credential requests are bearer and not affected.

Rejected: persisting every accepted nonce. It writes on every signed request
and only protects a 5-minute window.

### Expiry and replay

- Session expiry is already checked at use (`principal`) and on each open
  (`open_session` drops expired records). Option 2 keeps the same checks at load.
- Session tokens are bearer and are not nonce-protected in any option. A
  stolen session is valid until it expires, as designed.
- The signed-request guard is the only replay defence. Option 3 closes its
  restart gap without new persistence.

### Recommendation

**Option 3.** Keep sessions and codes volatile, and add the boot quiet period
so that a restart cannot reopen the replay window. The downside is a sign-in
after each restart, which is cheap for a personal server. If sessions must
survive deploys, add option 2 later as a separate store. Do not use option 1,
because it would put bearer state into the log that section 3 replicates.

---

## 2. Cross-server friend-envelope transport

### The problem

`receive_request` and `receive_accept` already check an envelope's signature
against the sender's account key, so a server does not need to trust the
sender's server. Nothing carries the envelope, and the receiving code is
exercised only by tests.

### Replay and failure findings

Read from `directory.rs`, not run:

- **Stale accept re-applies after a new request (serious).**
  `FriendAccepted` (`change`) requires only a pending request from `from` to
  `to`. It does not say which request. Sequence: Ann asks Bo, Bo accepts and
  the accept envelope A1 is captured. Bo removes Ann. Ann asks again, so a new
  request is pending on Ann's server. Replaying A1 to Ann's server now creates
  a friendship Bo did not accept again. Anyone who sees the envelope in transit
  (manual copying, a plain HTTP push, a log) can do this.
- **Stale request re-creates a pending request (minor).** `FriendRequested`
  refuses only when the pair is already friends or a request is pending. After
  a removal, an old request is accepted again and goes back to pending. The
  recipient must decide again, so this is a nuisance, not a takeover.
- **No expiry.** `verify` ignores `at`, so envelopes never expire.
- **No audience.** An envelope does not name the server it is for. An account
  key can exist on two servers (keyed sign-up), so one envelope is valid on
  both.
- **No delivery state.** The sender journals `FriendRequested` when it asks,
  and nothing records whether the envelope arrived. The view cannot say "not
  yet delivered".

### Trust

The account key is the only authority an envelope carries. What a receiving
server can know is "account X asked or accepted", not "server Y delivered it".
So there is no set of server keys to accept or reject for correctness. A server
allow-list would be an abuse control only.

Required checks on receipt, in every option:

1. The signature verifies against `from` (existing).
2. `to` is a local account (existing).
3. The envelope names this server as its audience (new).
4. `at` is within an expiry, for example 30 days (new).
5. The envelope ID has not been applied before. The set is durable and kept in
   the directory journal (new).
6. An acceptance names the request it accepts, and that request is pending
   (new, and it closes the stale-accept case).

Checks 3 to 6 need a changed envelope body. Nothing is deployed across servers
yet, so the change costs only the existing tests.

### Options

1. **Push over HTTP to the recipient's server.** The sender's server POSTs the
   envelope JSON to a new endpoint on the recipient, from a durable outbox.
   - The recipient address comes from a contact card, which is not built.
   - TLS authenticates the host. It is not needed for correctness, because the
     signature does that.
   - Failure: retry with backoff while the envelope sits in the outbox. Stop on
     a 2xx, or on a permanent refusal (bad signature, wrong audience, unknown
     account), which is not retried and is shown to the sender.
   - Receipt is idempotent. An envelope already applied returns success.

2. **Pull by URL.** The sender publishes each envelope at an unguessable URL,
   and the recipient fetches it.
   - The recipient needs a notification or a poll, and the sender must stay
     online until the fetch.
   - The URL is effectively a bearer secret, and the envelope reveals the
     friendship to anyone who learns it.

3. **Manual, through the explorer.** Add a `receive` action to the grammar
   (`parse_line` and `action_from_fields`), and let an operator paste the
   envelope into the other server's explorer over SSH or HTTP.
   - No new network surface, and the receiving path is the one that will be
     used in option 1.
   - Needs a person for every delivery.

4. **Keep copying by hand, with no change.** Works for a few friends. It keeps
   the stale-accept replay open until the checks above land.

### Removal

A removal is one-sided today, so the two servers diverge. The options are to
send a removal envelope (a new kind), to leave it one-sided, or to make removal
a request and not a state change. This needs a decision (see the list at the
end). It does not block the transport.

### Recommendation

**Option 1, push over HTTPS from a durable outbox**, with the six checks above.
Ship option 3 first, because it exposes `receive_request` and `receive_accept`
through the explorer and tests the receiving path before any network push
exists. The outbox records each envelope until it is delivered, so a restart
does not lose it, and the view can show "not yet delivered". Option 2 is a
weaker substitute for option 1, and option 4 leaves the replay open.

---

## 3. Directory replication between servers

### The problem

Each server applies its own changes through `apply`. Two servers that accept
writes will produce journals that refuse each other's entries, and some of the
refusals break invariants. Concretely, the current code gives:

- **Group IDs.** Each server creates `GroupCreated { id: next_group }`. Two
  servers each create group 1, and the second is refused as out of sequence.
- **Keys and emails.** Two servers can each attach the same key or confirm the
  same email to different accounts. One must lose.
- **Admins.** Two admin removals, each valid alone, can together leave a group
  with no admin. `ensure_admins` runs on each apply, not on a merge.
- **Nesting.** Two nested-group inserts, each valid alone, can make a cycle
  (`reaches`).
- **Membership and rename.** Two `MemberSet` or `GroupUpdated` entries
  conflict by order alone.
- **Friend removal.** Already divergent, as noted in section 2.

A correct merge has to decide, deterministically, which change wins and which is
refused, and every server has to reach the same answer.

### Options

1. **Single authority per directory.** One server is the writer for a given
   directory. Others hold read-only copies or forward writes over the section 2
   transport.
   - The apply path is unchanged and there is no merge rule. Group IDs stay
     sequential.
   - Cost: writes need the authority online, and the authority is a single point
     of failure. A second server can accept friendships only by forwarding them.

2. **Signed append-only log with a deterministic order.** Each change becomes a
   record: the `DirectoryEntry`, the actor's Ed25519 signature, the author
   server's ID, a Lamport clock, and the IDs of the heads it saw. Merged order is
   `(lamport, author, entry hash)`. Each server rebuilds state by running the
   sorted log through `apply`.
   - Account actions are signed at the time they happen, with the key derived
     from the presented credential (Decision 3 keeps no key). Server actions are
     signed with the server's key, which needs a stable server identity.
   - Entries the author accepted but a replica refuses (the second admin
     removal, a duplicate email) are kept in the log and skipped the same way on
     every replica. The author sees the refusal when it syncs.
   - Group IDs must come from the entry, not from a counter. Legacy journals keep
     their sequential IDs.
   - Sync needs a protocol to exchange heads and missing entries. That protocol
     is not designed here.

3. **CRDT-style state merge.** Friends as an OR-set, memberships and names as
   last-writer-wins registers, deletes as tombstones.
   - Converges without an order, and never refuses.
   - It cannot enforce the invariants: the admin rule, key and email uniqueness,
     sequential or cycle-free nesting, or level rules. Enforcing them needs
     coordination, which a CRDT avoids. The rules are the point of Decisions 5
     to 7, so this option would replace the model.

### Recommendation

**Option 2, the signed log with a deterministic order.** It keeps both servers
writable, which is what the Phase 12 gap describes. It reuses `apply` and
`replay`, so the invariants are enforced by one code path. The merge is a sort
followed by a replay, which the journal already does.

Costs to accept: a merge replays the whole log (fine at this scale, and no
compaction exists today anyway); acknowledged writes can be refused on merge,
so the explorer must say so; the Lamport clock must persist with the journal;
the journal format changes (serde defaults keep old lines readable); and a sync
protocol is needed, which depends on the section 2 transport.

Option 1 is smaller and is the right choice if the servers do not need to write
the same directory. Choose it if decision 1 below is "no".

Privacy and identity: the log carries public keys, digests and emails.
`AccountSignedUp` already journals a digest, so replication would let a
credential issued on one server authenticate on another. That is probably
desirable, but it is a decision, not a side effect. Session tokens and codes must
stay out of the log in every option (section 1).

---

## Recommendations, one line each

- **Section 1, sessions:** keep sessions and codes volatile, and refuse signed requests for 10 minutes after boot. Persist only if sessions must survive deploys, and then in a separate store.
- **Section 2, envelope transport:** push over HTTPS from a durable outbox, with audience, expiry, a durable envelope-ID set, and acceptance that names its request. Ship the manual `receive` action first.
- **Section 3, replication:** a signed append-only log merged by `(lamport, author, hash)` and replayed through `apply`; single-authority only if concurrent writes are not needed.

## Decisions the user must make, most important first

1. **Must the same directory be writable on two servers?** If not, section 3 is out of scope or reduces to single authority, and section 2 is enough.
2. **Are accounts portable across servers, or local to one?** This decides whether keys and digests replicate, and whether a credential from one server works on another.
3. **Which servers may deliver friend envelopes?** Any server with per-account limits, or an allow-list of peer servers.
4. **Where does a recipient's server address come from?** A contact card (not built yet), or typed in by the user.
5. **Do emails replicate, or only the verified flag?** Replicating emails spreads personal data to every peer.
6. **What happens on friend removal across servers?** Send a removal envelope, leave it one-sided (as today), or treat it as a request.
7. **Do sessions survive restarts?** Recommended: no, with the boot quiet period. Yes requires a separate store.
8. **If section 3 is option 2, are group IDs changed to entry-derived IDs?** This is a journal format change, and existing sequential IDs stay valid.

## Open questions

- Does each server have a stable identity (for example its Iroh node ID) across restarts? Audience binding and server-signed entries depend on it.
- Should an envelope expire after 30 days, or after a shorter window?
- Should a pending request be capped per sender, to limit spam once delivery is open?
- Where should the outbox live: the directory journal, or a separate `outbox.jsonl`?

## Verification plan (for the implementation that follows)

- A test that a stale accept after a new request is refused (section 2). It should fail on the current code before the fix.
- A test that a request replayed after removal is refused or deduplicated.
- A test that an envelope naming another server is refused.
- A test that a pre-restart signed request is refused for 10 minutes after boot.
- A test that the journal, and any new log, never holds a code digest or a session token.
- For section 3: two servers that create conflicting changes converge to the same state after exchanging logs, and every refusal matches on both.

## References

- Phase 12 limitations: [accounts, groups, contacts and delegation](../2026-10-09T12-00-00Z_feature_directory_accounts_groups/2026-10-09T12-00-00Z_feature_directory_accounts_groups.md)
- Attenuated capabilities: [design](../2026-10-09T09-59-07Z_design_attenuated_capabilities/2026-10-09T09-59-07Z_design_attenuated_capabilities.md)
- Code: `pkgs/id/src/directory.rs`, `pkgs/id/src/directory_auth.rs`, `pkgs/id/src/directory_view.rs`, `pkgs/id/src/world.rs`, `pkgs/id/src/world_hub.rs`, `pkgs/id/src/world_store.rs`
