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
12. **Mail goes to an outbox, a command, or a loopback relay.** `--world-mail-outbox <DIR>` writes
    each message to a 0600 file. `--world-mail-command <PROGRAM>` runs the
    program with `ID_MAIL_TO`, `ID_MAIL_SUBJECT` and `ID_MAIL_BODY` in its
    environment, with no shell. Without either, email actions are refused and
    only an admin can verify an account. `--world-mail-smtp` adds a loopback
    relay (Phase 10). The three transports are mutually exclusive.
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
group screens as operations, envelope signing with verification, public-key
accounts, email confirmation and sign-in, signed HTTP requests, and the mail
sinks. The list of what is not built is in Phase 12.

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

### Phase 6: keys, email, sign-in, mail and signed requests

Code: `directory.rs`, `directory_view.rs`, the new `directory_auth.rs` and
`directory_mail.rs`, `world.rs`, `world_hub.rs`, `world_session.rs`,
`web/world_ws.rs`, `world_ssh.rs`, `commands/serve.rs`, `cli.rs`, `main.rs`,
`lib.rs`.

- Keys. An account can hold several Ed25519 keys. Sign-up can take a key, and
  then the account ID is that key and no secret is issued. A key is attached
  by a request signed with that key (the key-proof rule), so no one can claim
  a key they do not hold. A key belongs to one account, and a second account
  cannot take it. Keyed accounts need no email.
- Email. An address is normalized (trimmed, lowercased, checked for shape and
  length) and belongs to one account. Adding one sends a 6-digit code, and
  confirming the code records the address. Emails are only stored once
  confirmed, so the `emails` set is the confirmed set.
- Sign-in by email. `SignInEmail` sends a code to a known address. The
  response is the same whether or not the address belongs to an account.
  `ConfirmSignIn` opens a 12-hour session for the holder.
- Verified tier. An account is verified when an admin verified it or when it
  holds at least one confirmed email. The admin flag and email confirmation
  are separate facts, and the tier uses either one.
- Caller proof. Requests carry a Bearer session (`sess.<hex>`), a credential,
  the admin token, or a signed request (`x-id-key`, `x-id-at`, `x-id-nonce`,
  `x-id-signature`). The signed message is
  `id-request-v1\n{METHOD}\n{path+query}\n{at}\n{nonce}\n{sha256(body) hex}`.
  It must be within ±5 minutes, and each nonce is accepted once.
- Sessions and codes are held in memory only. Codes expire after 15 minutes,
  allow 5 attempts, and can be resent after 60 seconds. Only digests are kept.
- Mail. `MailSink` is a synchronous trait called from `spawn_blocking`. A
  request that needs mail is refused with 403 when no sink is configured. The
  `DirectoryOutcome` mail field is skipped by serde, and its Debug output is
  redacted.
- Transports. HTTP: `GET` and `POST /api/world/directory`. WebSocket and SSH
  reuse the same `directory::run`. SSH public-key login is accepted only for
  `explore:` users, and russh checks the signature before the handler runs.
- Status mapping over HTTP: Unauthenticated 401, Forbidden 403, NotFound 404,
  Conflict 409, RateLimited 429, anything else 400. Success responses carry
  `Cache-Control: no-store`. This replaces the Phase 5 note that every
  directory error was a 400.

Verification:

- `cargo test --features "world ssh web" --lib`: 808 passed.
- `cargo test --lib` (default features): 623 passed.
- `cargo build --no-default-features`: no warnings.
- `cargo fmt --check`: clean.
- Clippy 1.97.0 reports nothing in the new files. The `directory_view.rs` test
  helper no longer takes `&mut`. The remaining diagnostics are in code that
  was already at HEAD.

Tests added in this phase cover: sessions, single-use codes, attempts and
expiry, resend limits, signed requests verified once, binding of a signature
to method, target, body and time, keyed sign-up, key ownership and uniqueness,
email confirmation (Server only), email removal, malformed email, sign-in
by mail through the view, a signed HTTP sign-up that opens a session, a reused
nonce refused with 401, outbox mode 0600, command environment passing, and a
failing command returning an error.

Choices made:

- Keyed accounts. The account ID is the public key, so a keyed account has no
  secret to lose.
- Key-proof rule. Only a request signed by a key can attach that key.
- Verified tier from email or admin, with enumeration-resistant sign-in.
- No shell in the command sink. The message is passed through environment
  variables.
- Typed refusals with a fixed status mapping, and 400 as the default.

Not built, and carried forward:

- Real SMTP and Cloudflare Worker mail sinks. The sink trait is the seam for
  them.
- Admin invite bounds (`uses`, `expires_in_secs`).
- The `delegate_to_friend` WebSocket frame and its `WorldHandle` method.
- The HTML explorer (`/explore`, login, logout, `me`, groups, `act`), with CSP,
  escaping, and a test that HTML and JSON show the same entities. Forms would
  need `SameSite=Strict` cookies for CSRF.
- The SSH interactive explorer with commands and `quit`. The current explorer
  is one-shot and read-only.
- A test of the SSH public-key login.
- An end-to-end HTTP test of the mail sign-in path.
- CLI tests for `--world-mail-outbox` and `--world-mail-command`.
- Group and friend actions are in `run` but are not tested over HTTP or SSH.
- Replication of the directory between servers, and transport of friend
  envelopes (they are still copied by hand).
- Per-export module gating.
- Scratch-server run.

Known limitations:

- Sessions, codes and replay nonces are in memory only, so a restart signs
  everyone out and forgets pending codes. Replay protection is the time
  window plus an in-memory nonce set.
- The directory is per world (`directory.jsonl` in the world folder), though
  Decision 1 said one per server. Moving it to the server is not done.
- `AddEmail` tells a signed-in user whether an address belongs to another
  account. Sign-in does not reveal this.
- A sign-in mail can be silently dropped by the resend limit. The response is
  the same either way.


### Phase 7: interactive SSH explorer

Code: `directory_view.rs` (`Line`, `parse_line`, `action_from_fields`, `HELP`),
`world_ssh.rs` (`Explorer`, `run_explorer`, `Typing`), `world_session.rs`
(`DirectoryOutcome::mailed`).

- One grammar. `parse_line` turns a typed line into the same `DirectoryAction`
  that `action_from_fields` builds from HTML form fields, so the text, JSON
  and HTML transports share one set of actions.
- Interactive explorer. `explore:` shells open a prompt (`id> `) and stay
  open until `quit`, `exit`, Ctrl-C, or Ctrl-D on an empty line. Input is
  line-edited (backspace), escape sequences are dropped, and CR, LF and CRLF
  each submit one line.
- Caller switching. `signup`, `keysignup` and `session` replace the caller
  with the new credential or session, and `signout` drops back to the
  connection's key. Any change of caller drops the admin token.
- Mail feedback. When a mail was handed to the sink, the explorer prints
  `a code was mailed to ADDR`. The code itself is never printed.

Verification:

- `cargo test --features "world ssh web" --lib`: 815 passed.
- `cargo test --lib`: 626 passed.
- `cargo build --no-default-features`: no warnings.
- Clippy reports nothing in `directory_view.rs` or `world_ssh.rs`.

Tests added: the explorer shows the directory to the admin and to anonymous
viewers and closes on `quit`; sign-up, help, unknown commands and usage
errors; friend request, accept and remove, and group membership over SSH; a
public-key login that signs up and is recognised on reconnect, and an unknown
key that sees only the anonymous view; the line editor; the grammar tests in
`directory_view`.

Choices made:

- The grammar lives in one module, shared by every transport.
- Caller switching is per connection, so a shell cannot keep an identity its
  login did not give it.

Not built in this phase: nothing from the Phase 7 list remains open.

### Phase 8: HTML explorer

Code: `web/explore.rs` (new), `web/world_ws.rs` (`world_routes` merges the
explorer; `refusal_status` shared), `web/templates.rs` (`html_escape` shared),
`directory_view.rs` (`level_name` shared).

- Routes. `GET /explore` shows the view and the forms for the viewer.
  `POST /explore/login` takes a credential or the admin token and opens a
  session. `POST /explore/act` takes the form fields that `action_from_fields`
  reads, so the forms and the typed grammar share one set of actions. Sign-out
  is an `act` form, so there is no separate logout route.
- Cookie. The browser holds only a `sess.` session token in `id_explore`, with
  `HttpOnly`, `SameSite=Strict`, `Path=/explore`, and the directory's 12-hour
  lifetime. A credential or admin login goes through `OpenSession`, and sign-up
  signs the browser in as the SSH explorer does. A 401 clears the cookie.
- Admin. The admin token is posted in the login form and exchanged for an admin
  session. It is not stored. Plain HTTP sends it in the clear, so the page
  tells the user to use HTTPS.
- Headers. Every explorer response carries
  `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'`,
  `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`,
  `Referrer-Policy: no-referrer`, and `Cache-Control: no-store`. The page has no
  script.
- Escaping. Every value that comes from the directory passes through
  `html_escape`. The `Origin` check in the web guard covers CSRF for form posts,
  and `SameSite=Strict` covers the cookie.
- Key sign-up is not offered in the browser, because a form cannot prove a
  key. Key-proof requests need a client that signs.

Verification:

- `cargo test --features "world ssh web" --lib`: 823 passed.
- `cargo test --lib`: 626 passed.
- `cargo build --features web` and `--no-default-features`: no new warnings.
- Clippy reports nothing in `web/explore.rs`.

Tests added: the page policy headers and no script; sign-up in the browser,
then sign-out clears the cookie; escaping of names and refused actions; a
credential login and a wrong admin token refused with 401; the admin session
with the verify form; an ended session refused and its cookie cleared; every
form builds the action it names; HTML, text and JSON show the same accounts
and groups, with HTML escaping the markup in names.

Not done: the explorer does not pick a world (it uses the default world); a
`Secure` cookie flag for TLS deployments is not set.

### Phase 9: invite bounds and friend delegation frame

Code: `world.rs` (`WorldCommand::Issue` carries bounds, new `DelegateToFriend`
command, `WorldHandle::issue_bounded` and `delegate_to_friend`, shared
`check_storage` and `record_issued` for the three mint paths),
`world_session.rs` (`invite_bounds`, `InviteError::InvalidBounds`, the
`invite` and `DelegateToFriend` frames, `send_delegation`),
`web/world_ws.rs` (`InviteRequest` fields, 400 on bad bounds).

- Invite lifetime. An invite now expires. The default is 7 days and the
  maximum is 30 days (`INVITE_DEFAULT_SECS`, `INVITE_MAX_SECS`). Before this
  change invites never expired, so this is a behaviour change. `uses` is
  optional, and `0` is refused. Zero or over-long lifetimes are refused with
  the message `expires_in_secs must be from 1 to 2592000 (30 days)`.
- Order of checks. The admin secret is checked before the bounds, so a wrong
  token gets 401 whatever the body says. Bad bounds on HTTP return 400 with an
  `error` field. On the frame they return an `error` frame. The mail install
  path maps them to `InstallError::InvalidUpload`, which is unreachable there
  because the install path always passes default bounds.
- `delegate_to_friend` frame. A joined session whose capability belongs to an
  account may send `{"type":"delegate_to_friend","friend":…,"name":…,"scopes":[…]}`
  with optional `expires_at` and `uses`. The core rules are unchanged: the
  friend must be a friend, the friend's ceiling must hold the scopes, and the
  friendship is checked on each use. Any refusal returns `delegation denied`.
  Unknown scopes return `unknown scope`.
- Shared reply. `attenuate` and `delegate_to_friend` use one `send_delegation`,
  so the response shape is the same for both.

Tests added: `invite_lifetimes_default_to_a_week_and_stop_at_thirty_days`;
`invite_frame_refuses_bounds_out_of_range`;
`delegate_to_friend_frame_needs_an_account_capability`;
`invite_bounds_are_checked_after_the_admin_secret` (HTTP 400 for each bad
bound, 401 for a wrong token with a bad body, 200 for valid bounds);
`a_friend_delegation_through_the_handle_is_held_to_the_friendship`;
`a_bounded_issue_stops_after_its_uses`.

Also fixed: `account_tests` in `world.rs` lacked the test `allow` attribute, so
`clippy --all-targets` reported its `unwrap` calls as errors. It now has the
attribute like the other test modules.

Verification:

- `cargo test --features "world ssh web" --lib`: 829 passed.
- `cargo test --lib`: 631 passed.
- `cargo fmt --check` clean. `cargo build --features web` and
  `--no-default-features` produce no warnings.
- Clippy reports no errors. The remaining warnings in the touched files
  predate this phase.

Not done: the `id` CLI has no `--expires-in` or `--uses` flags for `invite`,
so the bounds are reachable over HTTP and the frame only.

### Phase 10: SMTP sink, mail flags and mail sign-in over HTTP

Code: `directory_mail.rs` (`SmtpSink`, dot-stuffing, reply checks),
`cli.rs` (`--world-mail-smtp`, `--world-mail-from`, conflicts and requires),
`commands/serve.rs` (`ServeOptions` fields; `mail_sink` takes the SMTP settings
and returns `Result`), `main.rs` (passes the flags through),
`web/world_ws.rs` (end-to-end test).

- Transport. `SmtpSink` opens a plain TCP connection to a relay and speaks
  `EHLO`, `MAIL FROM`, `RCPT TO`, `DATA`, `QUIT`. It does no TLS and no login.
  The relay on loopback has to forward the mail with TLS, which a local MTA
  such as Postfix can do. The sink does not reach a remote server.
- Loopback only. `SmtpSink::new` refuses any relay address that is not
  loopback, so an operator cannot point the world at a remote server with
  unauthenticated plain SMTP by mistake. `serve` fails at start-up.
- Injection. Recipient and sender must be plain addresses: ASCII graphic
  characters, with an `@`, and none of `<`, `>`, `,`, `;`, `"`, `\`. The subject
  must have no control characters. These checks run before the connection is
  opened, so a refused message never reaches the relay.
- Body. Line endings become CRLF and a line that starts with `.` is doubled.
  The message ends with `.` on its own line, so a body cannot end the message
  early.
- Replies. Every reply is checked against the expected code, and multi-line
  replies are read to their last line. A refused recipient is an error and no
  `DATA` is sent. Timeouts are 10 seconds.
- Flags. `--world-mail-smtp <IP:PORT>` requires `--world` and
  `--world-mail-from`. `--world-mail-from` requires `--world-mail-smtp`. The
  outbox, command and SMTP transports conflict with each other, so exactly one
  is chosen.
- Limits. Non-ASCII addresses are refused, so internationalised addresses do
  not work yet. The sink does not add `Date` or `Message-ID`; the relay does.
  The relay's reply text appears in error messages.

Tests added: `the_smtp_sink_speaks_plain_smtp_and_stuffs_dots` (an in-process
fake relay records the transcript); `a_refused_recipient_is_an_error_and_no_body_is_sent`;
`the_smtp_sink_refuses_remote_relays_senders_and_injection_before_connecting`;
`mail_sink_picks_one_transport_and_refuses_an_unusable_relay`;
`test_cli_parse_world_mail_flags` (valid pair, each missing half refused, each
conflict refused, the admin token and `--world` required, a bad address
refused); `a_mailed_code_confirms_an_address_and_signs_in_over_http` (sign up,
add an email, the code is mailed to the outbox and confirms it, then a sign-in
code is mailed, traded for a session, and the session reads as the same
account).

Verification:

- `cargo test --features "world ssh web" --lib`: 835 passed.
- `cargo test --lib`: 636 passed.
- `cargo fmt --check` clean. `cargo build --features web` and
  `--no-default-features` produce no warnings.

Not done in this phase: a Cloudflare Worker sink (see Phase 12).

### Phase 11: group and friend actions over HTTP, scratch-server run

Code: `web/world_ws.rs` (test helpers `act`, `view_as`, `named`, `signed_up`,
and two tests). No product code changed.

Tests added:

- `groups_are_managed_and_their_levels_gate_each_change_over_http`: create a
  group, add a member at `read`, the member sees the level and the members. A
  `read` member gets 403 on a rename and on `set_public`. An anonymous viewer
  sees no groups until an admin makes the group public, then sees it with
  `members` null. Delete removes it from the anonymous view.
- `friend_requests_are_sent_accepted_and_ended_over_http`: an anonymous request
  gets 401. A request shows as `incoming` for the target and `outgoing` for the
  sender. Acceptance makes both `friends`. Removal ends the friendship at both
  ends.

Scratch-server run. The server ran on `127.0.0.1` with `--ephemeral
--no-relay --no-gossip --no-mdns --web --world --world-name lobby
--world-admin-token ... --world-mail-outbox DIR`. Checked over curl:

- Sign-up returns a credential. Group creation, `set_member` at `read`, and
  the 403s for rename and `set_public` match the tests.
- Making the group public shows it to anonymous viewers. Deleting it removes it.
- Friend request, acceptance and removal behave as in the test.
- `add_email` wrote one 0600 `.eml` file, and `confirm_email` with the code from
  that file made the account `verified: true`.
- `/explore` returns 200 and lists the accounts. A request with a foreign
  `Host` header gets 421.

Observed, not changed: with `--no-relay --no-gossip --no-mdns`, the Iroh
endpoint still made outbound HTTPS connections to `dns.iroh.link` for pkarr
publishing. The flags do not turn that off. The server was stopped and the
scratch directory removed.

Verification:

- `cargo test --features "world ssh web" --lib`: 837 passed.
- `cargo test --lib`: 636 passed.
- `cargo fmt --check` clean. `cargo build --features web` and
  `--no-default-features`: no warnings.
- Clippy reports nothing in the new tests. The two warnings left in
  `world_ws.rs` (a `dyn` clone and a `usize` cast in older tests) predate this
  work.

Not done in this phase: nothing from the Phase 11 list remains open.

### Phase 12: corrected not-done list, deferrals and limitations

This replaces the not-done lists in Phases 5 and 6. Those lists are kept above
as history.

Closed since they were written:

- Admin invite bounds (Phase 9).
- The `delegate_to_friend` frame and its `WorldHandle` method (Phase 9).
- Real SMTP: a loopback relay sink (Phase 10).
- An end-to-end HTTP test of mail sign-in (Phase 10).
- CLI tests for the mail flags (Phase 10).
- Group and friend actions tested over HTTP (Phase 11) and over SSH (Phase 7).
- The scratch-server run (Phase 11).
- The SSH public-key login test (Phase 7).

Still not done, and deferred:

- **Cloudflare Worker mail sink.** The `MailSink` trait is the seam. Until it
  exists, `--world-mail-command` can reach a Worker over HTTP. This example is
  not run or tested:

  ```sh
  #!/bin/sh
  # Run by id for each mail; the endpoint and token come from the environment.
  curl -sf -X POST "$WORKER_MAIL_URL" \
    -H "authorization: Bearer $WORKER_MAIL_TOKEN" \
    --data-urlencode "to=$ID_MAIL_TO" \
    --data-urlencode "subject=$ID_MAIL_SUBJECT" \
    --data-urlencode "body=$ID_MAIL_BODY"
  ```

  Deferred because it needs a Worker, a sending domain, and a decision on
  where the token is kept. None of these is in the repo.
- **Per-export module gating.** Deferred. The attenuated-capabilities design
  (open question 4) recommends gating per module first, and per export only
  if needed. Nothing here changes that.
- **Directory replication between servers.** Deferred. Two servers can both
  accept friendships, change membership and delete groups, so replication
  needs a merge and conflict rule first. That is a design question, not a
  small addition.
- **Cross-server friend-envelope transport.** Deferred for the same reason.
  `receive_request` verifies an envelope signed elsewhere, but nothing
  delivers one. Envelopes are still copied by hand.
- **`id world invite` bounds flags.** The CLI has no `--expires-in` or
  `--uses`. Bounds are reachable over HTTP and the WebSocket frame only.

Corrections to earlier sections:

- Decision 12 said the server had no SMTP client. Phase 10 added one, and
  the text now says so.
- The Phase 5 "scratch-server run: not done" is closed by Phase 11.
- Decision 1 (one directory per server) is not met. The directory is per
  world, as the limitation below says.

Known limitations, carried forward:

- Sessions, codes and replay nonces are in memory only. A restart signs
  everyone out and forgets pending codes. Replay protection is the time window
  plus an in-memory nonce set.
- The directory is per world (`directory.jsonl`), not per server as Decision 1
  says.
- The HTML explorer uses only the default world and sets no `Secure` cookie
  flag. A TLS deployment needs that flag added. The admin token typed into the
  form is sent in the clear over plain HTTP.
- `SmtpSink` refuses non-ASCII addresses, so internationalised addresses do not
  work yet.
- Invites are now bounded: they expire after 7 days by default and at most 30
  days. This is a behaviour change for HTTP and frame invites. Invites made
  directly through `WorldHandle::issue` are unchanged.
- With `--no-relay --no-gossip --no-mdns`, Iroh still publishes to
  `dns.iroh.link` (Phase 11).

Choices made across the work:

- One grammar for all transports. `parse_line`, `action_from_fields` and the
  JSON form all build the same `DirectoryAction`, so the text, JSON and HTML
  views cannot drift apart.
- SSH caller switching is per connection. `signup`, `keysignup` and `session`
  change the caller only for that connection, and `signout` returns it to the
  key it logged in with. A shell cannot keep an identity its login did not
  give it.
- `DirectoryOutcome::mailed` tells a transport that a code was sent, without
  carrying the code. The SSH explorer prints the address and never the code.
- Invite default expiry is 7 days and the maximum is 30 days. This is a
  deliberate change, listed above.
- Loopback-only SMTP. A remote relay with plain SMTP and no login is refused at
  start-up.
- No new crates. Everything uses the existing dependencies.
- The Worker sink is deferred, not stubbed.
- The HTML explorer does not offer key sign-up, because a form cannot prove
  possession of a key. Key sign-up needs a client that signs.

---

## 2026-10-10T02-37-54Z Implementation: explorer cookie flags

The explorer's session cookie (`id_explore`) already had `HttpOnly` and
`SameSite=Strict`, but no `Secure` flag, and the admin token form sent the token
over whatever scheme the server speaks. The server speaks only plain HTTP; TLS
is terminated by a proxy in front, which the server cannot see. This supersedes
the Known limitations item about the missing `Secure` flag: the flag now exists,
but a TLS deployment still has to terminate HTTPS and pass it.

Choices:

- An opt-in flag, `--web-cookie-secure` (env `ID_WEB_COOKIE_SECURE`), default
  off. The operator sets it when HTTPS is terminated in front. It is off by
  default because browsers drop `Secure` cookies over plain HTTP except on
  localhost, so a default-on flag would silently break sign-in on a plain-HTTP
  LAN address.
- The scheme is not inferred from headers. `X-Forwarded-Proto` is not read,
  because a client can set it unless a trusted proxy strips it.
- With the flag, `Secure` is added to every explorer `Set-Cookie`: sign-in,
  sign-out and the clear sent on a 401. Without it the attribute is absent.
- Startup warning. When a hosted world is enabled (so the admin form is
  served), the bind is not loopback, and the flag is not set, `serve` prints a
  warning that the token and session cookie travel in clear unless HTTPS is
  terminated in front.

Scope: explorer cookies only. The `id_token` cookie in `web/security.rs` is also
set without `Secure`, and is not changed here.

Changed: `cli.rs` (flag), `main.rs` and `commands/serve.rs` (option and warning),
`web/mod.rs` (`cookie_secure` on `AppState`, passed to `web_router`),
`web/routes.rs` and `web/world_ws.rs` (`WorldWebState::cookie_secure`), and
`web/explore.rs` (cookie builders take `secure`).

Verification:

- `cargo test --features web --lib -- web::explore cli::tests`: 110 passed.
  New tests: `explorer_cookies_are_http_only_strict_and_not_secure_by_default`,
  `explorer_cookies_are_secure_when_https_is_terminated_in_front`,
  `test_cli_parse_web_cookie_secure`.
- `cargo clippy --all-targets --all-features` with the rustup 1.97.0 cargo:
  exit 0, with no new warnings in the changed code.
- `cargo test --all-features -- --skip serve_tests`: 837 passed. Three
  `world_compile` tests fail because `examples/roc-world` has no built
  `targets/wasm32/host.wasm`. They are unrelated to this change.
- `bun run test` (343 passed), `bun run typecheck` and `cargo doc` pass.
  `just ci` stops at `web-fmt-check`, which flags `web/src/world.ts`; this
  change does not touch `web/`.
- Live check, `id serve --ephemeral --web --world` on `0.0.0.0` over plain HTTP.
  Default: `Set-Cookie` has `HttpOnly; SameSite=Strict` and no `Secure`, and the
  warning prints. With `--web-cookie-secure`: `Secure` is on the sign-in and
  clear cookies, and the warning does not print. Loopback bind: no warning.
- Not verified in a browser. Cookie acceptance over HTTPS through a real TLS
  proxy was not tested.
