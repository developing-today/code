# Accounts, groups, contacts and delegation

Related: [attenuated capabilities](../2026-10-09T09-59-07Z_design_attenuated_capabilities/2026-10-09T09-59-07Z_design_attenuated_capabilities.md)
(the capability records, attenuation and the ceiling this builds on).

## Intent

Worlds today know participants only as capability holders. A person cannot
sign up, be placed in groups, befriend another person, or hand a friend a
narrower share of what they hold. The request:

- open sign-up with permission sets for three tiers: anonymous, registered,
  and email/identity-confirmed;
- groups with membership levels (access, read, write, manage, admin), an
  account in several groups, groups inside groups;
- a decentralised friend/contact system;
- delegation of granted permissions, with restrictions (narrower functions,
  earlier expiry, use limits);
- a personal user screen and a group screen;
- a user and group explorer on both SSH and HTTP, showing the same data.

Open items from the attenuation design (expiry and use limits, the tick
attribution decision) are included.

## Decisions

1. **One directory per server.** Accounts, groups and contacts belong to the
   server, not to one world. Worlds keep their own ceilings and capabilities.
2. **Accounts are self-certifying.** An account's ID is the hex of an Ed25519
   public key. Signing up draws a 256-bit secret; the signing key is derived
   from it with domain separation (`SHA-256("id-account-v1" || secret)`), and
   only the public key and a SHA-256 digest of the secret are stored. The
   credential shown once is `acct.<id>.<secret-hex>`.
3. **Credentials are bearer, as capabilities are.** A presented credential is
   checked against its digest in constant time. Account operations that need a
   signature (friend requests, acceptances) are signed by the server with the
   key derived from the presented credential, and the key is not kept.
4. **Tiers are built-in groups with permission sets.** `anonymous` covers
   sessions with no account. `registered` covers signed-up accounts.
   `verified` covers accounts whose email or identity was confirmed. Every
   account is in `registered`, and in `verified` once confirmed. Admins edit
   the sets. Defaults match what guests get today, so existing worlds do not
   change behaviour.
5. **Groups and tiers are a ceiling, not a grant.** An account's ceiling is
   the union of its tier set and the sets of every group it belongs to,
   transitively. A participant's effective functions are
   `capability.functions ∩ world.ceiling ∩ account.ceiling`. Groups can narrow
   what a capability allows, and they can never widen a capability. A
   capability with no account uses the anonymous set.
6. **Group levels.** Ordered `access < read < write < manage < admin`:
   - `access`: sees the group and its own membership.
   - `read`: sees members, their levels, and the permission set.
   - `write`: edits the group's description.
   - `manage`: adds and removes members, and sets levels up to `write`, only
     for members below `manage`.
   - `admin`: all of the above, sets any level, changes the permission set and
     name, and deletes the group.
   A group must always keep at least one admin account, directly or through a
   nested admin group.
7. **Nested groups.** A group may be a member of another. Cycles are refused.
   A member group's contribution to a parent is its permission set, and its
   members' effective level in the parent is the minimum of the level the
   group holds and their level in the member group.
8. **Friends are signed envelopes.** A friend request, its acceptance, and a
   contact card are JSON bodies with an Ed25519 signature over the canonical
   body bytes. A recipient verifies them against the public key inside the
   envelope, so the envelope can travel through any channel and be checked
   without contacting the sender's server. Nothing in this is an authority
   decision (C1).
9. **Delegation goes to friends.** `attenuate` may name a recipient account.
   The recipient must be a friend of the delegator, and the holder's account
   ceiling must allow `delegate`. The child may narrow functions, set an
   earlier expiry, and set a use limit no larger than its parent's remaining
   budget.
10. **Expiry and use limits on capabilities.** `Issued` gains optional
    `expires_at` (Unix ms) and `uses`. A use is a committed action by the
    capability's holder (chat or input). The count is rebuilt from committed
    events on replay, so no new journal event is needed. A use draws down the
    holder's budget and each ancestor's; a child cannot outlive or outspend its
    parent.
11. **Ticks use the ceiling only.** World-level ticks are not caused by a
    participant. They are checked against the world ceiling and nothing else.
    This is open question 1 of the attenuation design, now settled.
12. **Email confirmation is delivered by a command.** `--mail-command` is run
    with the recipient and code in environment variables, and nothing is logged
    to stdout. Without it, email confirmation is refused and only an admin can
    verify an account. Real mail delivery is not built into the server.
13. **One explorer model, two renderers.** A `DirectoryView` is built once from
    the directory and a viewer's rights. SSH renders it as text, HTTP renders it
    as HTML and JSON. Both transports parse commands into the same `DirOp`. A
    test checks that every entity in the view appears in both renderings.

## Visibility

- Admin token: everything.
- Account: itself, its groups, members of groups where it holds `read` or
  better, and its friends.
- Anonymous: account display names and public groups only.

## Scope of this slice

Built: the design, expiry and use limits, the account, group and contact model
with its journal, the explorer model with SSH and HTTP renderers, personal and
group screens as operations, and envelope signing with verification.

Not built: transport of friend envelopes between servers (they are copied by
the user), an Iroh replication of the directory, real mail delivery, and
per-export module gating.

## Verification plan

- Levels: each rule is tested for allowed and refused actors, including
  "manage cannot grant admin" and "last admin cannot be removed".
- Nesting: cycles refused; effective level is the minimum across the path.
- Ceiling: a group change narrows a live participant without reissuing the
  capability.
- Delegation: refused for non-friends, for `delegate` outside the account's
  ceiling, and when the child widens functions, outlives the parent, or
  exceeds the parent's uses.
- Uses: a chain of three uses draws down the parent's budget.
- Envelopes: a tampered body or a wrong key fails verification.
- Journal: no credential or email secret appears in the directory journal.
- Parity: the same entities appear in SSH text and HTTP HTML.

## Progress

### Phase 1: expiry and use limits on delegated capabilities

- `Issued` carries optional `expires_at`, `uses` and `used` (serde default, so
  older journals still load). `used` is written at checkpoint so counts survive
  compaction.
- Expiry is checked on every authorised operation, including reads, against
  the holder and each ancestor. Use limits are checked and spent only on
  committed chat and input, for the holder and each ancestor.
- Delegation refuses an expiry in the past or later than the delegator's, a
  use limit below one, and a use limit above what the delegator's chain has
  left. Restore refuses a child that outlives its parent or has an empty use
  limit. Restore does not re-check use budgets, because the snapshot in a
  compacted journal can be later than the moment a child was issued.
- Ticks are gated only by the world's grant (`CapLedger`), never by a
  participant's capability. Test: `ticks_follow_the_world_grant_not_a_participant_capability`.
- Not in this phase: admin invites are still unbounded, and the delegation
  frame reports every refusal as "delegation denied".

### Phase 2: directory model

- `pkgs/id/src/directory.rs` is a pure model. Every change is a
  `DirectoryEntry`; live calls and journal replay both go through one `apply`,
  which runs on a copy and commits only if the copy keeps every group with an
  admin. A refused change leaves no trace, and a broken journal is refused on
  replay.
- Accounts: `acct.<id>.<secret-hex>`, where `id` is the hex Ed25519 key derived
  from SHA-256("id-account-v1" || secret). Only the SHA-256 digest of the secret
  is stored or journaled. Verification is `Actor::Server` only.
- Levels: `Access < Read < Write < Manage < Admin`. A group's admin may change
  anything. A manager may add and change members up to `write`, and may not
  change their own level or any member at `manage` or above. Write may edit the
  description only.
- Nesting: a nested group passes on at most the level it is held at, and the
  effective level is the highest across all paths. Cycles, including a group
  containing itself, are refused at insert.
- Ceiling: `tier ∪ scopes of every group the account holds`, where the tier is
  Anonymous, Registered or Verified. Defaults: Anonymous and Registered get
  GUEST; Verified gets GUEST|DELEGATE. The ceiling is not yet applied to
  participant effects; that is Phase 3.
- Friends: `Envelope` is Ed25519 over canonical JSON of kind, from, to and at.
  It is verified against the sender's ID, so a copied envelope can be checked
  on any server without asking the first one. A request is pending until the
  addressee accepts; a duplicate or reversed request is refused.
- Tests: 15 in `directory::tests`, covering credentials, secrets absent from
  entries, manager limits, the admin invariant, nesting and cycles, ceilings,
  tiers, friends on one server and across two, tampered and reattributed
  envelopes, and replay refusal.
- Not in this phase: no frames, no HTTP or SSH surface, no wiring into
  `World`, and no directory replication over Iroh. Journal files are appended
  by the caller; the server actor that will own them is Phase 3.

### Phase 3: accounts bound to capabilities, friend delegation

- `WorldCore` holds the `Directory`. A directory opened with `Directory::open`
  journals each accepted change before it is visible, so a refused change never
  reaches the journal.
- `issue_for_account` issues a capability whose `subject` is the account. Its
  scopes may not exceed that account's ceiling.
- `effective_scopes` replaces the stored scopes everywhere they are checked
  (`authorize`, delegation, and participant effects in `execute`). Along the
  whole delegation chain, each record's scopes are intersected with its
  subject's ceiling. Anonymous capabilities are unchanged.
- `delegate_to_friend` needs an account-bound capability, a current friend, and
  a friend ceiling that already holds the requested scopes. Friendship is checked
  again on every use, so ending a friendship stops the delegation.
- `subject` is journaled on `Issued` (serde default, omitted when absent), so
  an account binding survives restore and compaction. No secret is journaled.
- Tests: 4 in `world::account_tests`; 1 in `directory::tests` for journaling.
- Not in this phase: the frames and handle commands that expose these. Those
  are Phase 4.

### Phase 4: directory explorer over SSH and HTTP

- Groups have a `public` flag, default private. It is changed by
  `DirectoryEntry::GroupVisibility`, journaled like any other change, and
  `set_public` needs `Admin` on that group.
- `directory_view.rs` builds a `DirectoryView` from a `Directory` and a
  `Viewer` (`Anonymous`, `Account`, or `Admin`). `Viewer::resolve` lets the
  admin token win. A credential that names no account is refused unless the
  request is admin.
- Visibility is as stated above. Detail fields (scopes, friends, pending
  requests, verification) appear only for the account itself or for admin.
  Group members appear to a viewer holding `read` or better, and to admin.
  Anonymous viewers see account names and public groups only.
- `DirectoryAction` has two variants: `View`, and `SignUp { name }`. `run`
  returns a `DirectoryOutcome` with the view and, for sign-up, the credential
  once. After sign-up the view is the new account's view.
- HTTP: `GET /api/world/directory?world=` reads the view. `POST` takes the same
  action as a JSON body. The credential goes in `authorization: Bearer` and the
  admin token in `x-world-admin-token`. The view is nested under `view`. Any
  directory error is a 400 with `{"error": ...}`; a busy world is a 503; other
  lease errors are 404.
- SSH: log in as user `explore:<world>` (an empty world name is the default
  world). The password is a capability, or the admin token. The reply is
  `render_text` with CRLF line endings, then the channel closes. Refused logins
  wait `REFUSED_SHELL_DELAY`, as other refused shells do.
- Parity: both transports render the output of the same `run`. The test
  `json_and_text_name_the_same_accounts_and_groups` checks that the JSON and
  text renderings name the same entities. The SSH and HTTP tests each check
  their own transport. No test compares the SSH bytes with the HTTP bytes for
  the same viewer.
- Deviation from decision 13: there is no `DirOp`. Both transports take
  `DirectoryAction`. The HTML renderer is not built, so only the JSON and text
  renderers exist.
- Not in this phase: the HTML explorer; personal and group management screens;
  writes over SSH or HTTP beyond sign-up. Creating groups, adding members,
  setting levels, sending friend requests and making a group public exist in
  the model and are tested there, but no transport exposes them. Delegation
  frames are also not exposed. Scope above says "personal and group screens as
  operations", and that is only true of the model, not of the transports.

### Phase 5: verification and wrap-up

- `cargo fmt --check`: clean.
- `cargo test --features "world ssh web" --lib`: 791 passed.
- `cargo test --lib` (default features): 607 passed.
- Clippy 1.97.0, `--features "world ssh web" --all-targets`: no diagnostics in
  the lines this work changed. The remaining errors and warnings are in test
  modules and code that already existed at HEAD: `unwrap_used` in the tests of
  `world.rs`, `world_net.rs`, `world_hub.rs`, `world_native.rs`,
  `world_compile.rs` and `world_session.rs`, plus `clone_on_ref_ptr` and
  `cast_possible_truncation` in `web/world_ws.rs`.
- Scratch-server run: not done. The HTTP test binds `127.0.0.1:0` through axum,
  and the SSH test connects a real russh client to a real port. Those cover
  the transports, so a separate run was not added.
- Not done, and carried forward: real email delivery; delivery of friend
  envelopes between servers (they are copied by hand); Iroh replication of the
  directory; bounds on admin invites; per-export module gating; the HTML
  explorer; personal and group management screens; transport writes; delegation
  frames; verification by any route except the admin token; and finer HTTP
  error statuses (every directory error is a 400).
