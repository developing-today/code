# Key recovery for hosted ID users — Google SSO loss vs the ID keypair

**Date:** 2026-10-29 · **Issue:** [#2060](https://github.com/developing-today/code/issues/2060)
**Scope:** How a hosted-edition user recovers from (a) losing their Google account,
(b) losing their device / ID key, (c) using several devices, and (d) losing a single
device when social guardians exist — without the server ever holding a user key.
**Method:** Read-only review of `pkgs/id/src` (`store.rs`, `directory.rs`,
`directory_auth.rs`, `seitan/`, `invites/`, `encryption/`, `web/identity.rs`), plus
§4.3/§8 of `doc/2026-10-28T23-34-14Z_investigation_cloudflare_workers_fit/`.
No code was changed and no issues were filed; follow-on slices are listed at the end
for the gated synthesis issue to own.

---

## TL;DR verdict

**SSO can recover the hosted *account*. It cannot recover the *identity*, and it must
never be allowed to mint one.** Today the ID key is a raw 32-byte Ed25519 secret in a
local file with no export path and no backup: lose the device and the identity is gone,
which is the correct default and the whole problem. The recovery model that fits the
existing code is layered: device key (L0), passphrase-wrapped encrypted backup (L1),
k-of-n Shamir shares with key-proven guardians (L2), and Google SSO as an account
*pointer* that can never on its own rebind a key (L3).

| Case | What exists in the code today | Recoverable today? | Path that preserves LIST 7 |
| --- | --- | --- | --- |
| (a) Google account lost | **No SSO code at all** — no `google`, `oauth`, `sso`, `passkey` or `webauthn` in `pkgs/id/src` | n/a (design-only) | Key-native sign-in (`AccountKeyed`), or signed rebind from a surviving key; SSO alone may not rebind |
| (b) Device / ID key lost | `.iroh-key` is a raw key file, mode `0600`, created lazily, **no export or backup command** | **No** — identity is unrecoverable | L1 wrapped backup (ciphertext blob server-side) + optional L2 shares |
| (c) Multi-device | `Account.keys: BTreeSet` plus `AccountKeyAdded` / `AccountKeyRemoved` entries | **Partly** — the directory models it, the node does not | Per-device signer keys on one account; device certs signed by the identity key |
| (d) Social recovery | Seitan exists, but as **Keybase V2 invite tokens**, not safety numbers | **No** | k-of-n Shamir of the L1 unwrap key, guardians bound to their *account* keys |
| Server holds keys? | Account secret digest only (`SHA-256`), never a signing key | **Yes, preserved** | Reject custodial-key option §4.3.2 explicitly |

**Bottom line:** three of the four cases are unsolved today, and two of them (lost key,
no backup) are unsolved *by design* rather than by omission. The buildable core is small
and composable — an exportable encrypted backup, shares over an existing signed channel,
and a rule that SSO login never changes a key binding — and it should be built in that
order, because L1 alone converts "identity gone" into "identity recoverable".

---

## 1. What the code actually does today

The honest starting point is that recovery is not implemented, and the primitives that
would carry it are only half present.

### 1.1 The identity key is a bare file

- `lib.rs:292` — `pub const KEY_FILE: &str = ".iroh-key"`.
- `lib.rs:298` — `pub const CLIENT_KEY_FILE: &str = ".iroh-key-client"`; the client key is
  a *separate* key from the node key, loaded by `meta_client.rs:114`, `commands/client.rs:68`,
  `commands/world.rs:48` and `commands/id.rs:43`.
- `store.rs:91` — `load_or_create_keypair(path)` creates the file if absent; `store.rs:155`
  writes it with `.mode(0o600)`; `store.rs:179` tightens permissions on an existing file
  and warns if the OS refuses.
- `store.rs:64` — the file "contains just the 32-byte secret key in raw binary format".

So: two key files, both raw secret bytes, both per working directory, neither exportable.
`grep` for `export`, `backup` and `recover` across `pkgs/id/src` finds no key-export path —
the only `export` hits are `export_*` helpers unrelated to keys. This is the fact that
makes case (b) fatal: **there is no supported way to move an identity to a new machine**,
which also means there is no way to restore one.

### 1.2 The directory already has two account models, and one of them is right

`directory.rs:140-180` defines the durable entries:

- `AccountSignedUp { id, name, digest }` — "only the digest of its secret is stored"
  (`directory.rs:145`). `sign_up` (`directory.rs:561`) generates 32 random bytes, derives
  `id` from the public half (`directory.rs:573`), stores `SHA-256(secret)`, and returns
  `acct.<id>.<secret>` **to the caller exactly once**. That does **not** make the
  model noncustodial in effect (corrected per Codex review, PR #2278): the signing
  key is deterministically derived from the secret (`signing_key`,
  `directory.rs:1769`), and `authenticate` (`directory.rs:1432-1457`) reconstructs
  and returns that key every time the credential is presented —
  `prepare_request_friend` (`directory.rs:1130`) and `publish` (`directory.rs:1372`)
  then sign account objects with it. So the server-side code path *does* sign as the
  account whenever the bearer credential is presented, and anyone who saw the secret
  once — at creation, in a log, in transit — can re-derive the key and sign
  indefinitely. Digest-only persistent storage proves the secret is not *stored*; it
  does not establish an inability to sign. This exposure belongs to
  `AccountSignedUp` only; `AccountKeyed` (below) has no bearer credential to derive
  from.
- `AccountKeyed { id, name }` — "a new account whose ID is its own public key, so it has
  no secret: the key proves itself on each sign-in" (`directory.rs:155`). This is the model
  that satisfies LIST 7 without qualification, and the hosted edition should default to it.

`Account` (`directory.rs:78-93`) carries `id`, `name`, `verified`,
`keys: BTreeSet<String>` ("further public keys that sign in as this account") and
`emails`. `AccountKeyAdded` / `AccountKeyRemoved` (`directory.rs:166-178`) are durable
directory entries — **the multi-device and key-rotation primitive already exists at the
account layer**; what is missing is a rule about who may append those entries and what
proof they must carry.

### 1.3 Authentication: signed requests, one-time codes, bearer credentials

`directory_auth.rs` is the whole authentication story:

- `Caller` (`:60-90`) accepts one of four proofs: admin token, session (`sess.<32 bytes>`,
  `:119-121`), `acct.<id>.<secret>` credential (`:63`), or an Ed25519 signature.
- `verify_signed` (`:314-350`) checks a `id-request-v1` domain-separated message, a
  `SIGNED_WINDOW_MS = 5 * 60 * 1000` clock window (`:34`) and a nonce cache capped at
  `SIGNED_NONCE_LIMIT = 4096` (`:39`) held **in memory**. Sessions last 12h (`:24`); email
  codes 15 min with 5 attempts (`:26-29`).
- The module header is explicit: "Sessions, codes and replay records live in memory beside
  the world, so a restart signs everyone out and forgets pending codes."

Two consequences for recovery: (i) any recovery proof built on `verify_signed` inherits a
5-minute window and an in-memory replay cache — acceptable for a login, **not** acceptable
for a "replace my identity key" operation, which must be persisted and single-use; (ii)
there is no Google, OIDC, OAuth or WebAuthn code anywhere in `pkgs/id/src`. The SSO in
§4.3 of the Cloudflare investigation is a *design*, not an implementation, so everything
below about SSO is a specification for a thing that does not exist yet.

### 1.4 Seitan in this repo is invites, not safety numbers

The brief for this investigation assumed `pkgs/id/src/seitan/` is a safety-number /
social-verification mechanism. It is not: it is **Keybase Seitan V2 invite tokens**
(`seitan/mod.rs:1-17`). An 18-symbol token (`seitan/token.rs:10`, ~83 bits) is stretched
with scrypt (`seitan/derive.rs:3-9`, N=2^10, r=8, p=1) into a secret `SiKey`, from which
an invite ID and an Ed25519 keypair are derived; the joiner signs an `accept` with that
key and the admin admits it after checking revocation, expiry, single use, freshness
(default 5 minutes) and the signature (`seitan/admit.rs`, `seitan/freshness.rs`).

That is still highly relevant, for a different reason: **a Seitan invite is a proof that
the person on the other end holds a key derived from a out-of-band secret**. It is a good
channel for delivering a share, but it is a *bad* guardian identity — the derived key is
single-use and per-invite, so it cannot serve as a stable "this is Alice" anchor. Guardians
must be identified by their account key, bound at acceptance time.

Related surfaces: `invites/invitation.rs:17-25` parses an `id:join?group=…&ticket=…&by=…`
link where `by` is "the inviter's account key, the trust anchor for the invite event";
`petname.rs` keeps private local names with a 6-character fingerprint; `qr.rs` renders
invite text as SVG so a link can be scanned rather than pasted.

### 1.5 Group encryption already knows how to welcome a new device

`encryption/roster.rs:31` — "a member is its verifying key". `encryption/scheme.rs` drives
p2panda-encryption's DCGKA: adds carry a secret bundle, membership changes rotate the group
secret, and `encryption/welcome.rs` chunks a welcome envelope so a joiner can reconstruct
it. Practically: **adding a device as a group member is how a new device obtains the
decryption material for past epochs** — the mechanism exists, but it costs an epoch
rotation for every group the user is in, and it treats the device as a *person-shaped*
member rather than as one of several keys for a person.

### 1.6 The one place the server does hold keys (and why it is not a precedent)

`web/identity.rs:131` derives an Ed25519 signing key and an AES-256-GCM key from the
server's own secret, and uses the AES key to encrypt collaborative-editing client
identity records (`:33-37`, `:483-620`). That is the server protecting **its own** records
with **its own** key — legitimate, and unrelated to user identity. It must not be cited as
precedent for encrypting user key material server-side: the moment the server can unwrap a
user key, §4.3 option 2 (custodial) is back and LIST 7 is broken.

---

## 2. The constraint, stated precisely

LIST 7's model is "validly-signed objects plus allowed writers": an object counts because
a key signed it and that key was allowed to write there. Three things follow, and they
bound every design below:

1. The server may hold **public keys, digests, ciphertext, and routing metadata**. It may
   not hold a signing key or an unwrapped decryption key for a user.
2. A server-side operation may **record** a binding; it may not **create** one. A binding
   is a statement signed by the key being bound.
3. Recovering an account must not silently widen who can write. If SSO login can cause a
   new key to be accepted for an identity, then Google account takeover plus operator
   compliance equals identity takeover — the exact failure custodial keys were rejected for.

This gives the one-line rule the rest of the document hangs on:

> **SSO authenticates a person to the account record. It never authenticates a key to the
> identity. Rebinding a key requires a signature from a key that is already bound, or a
> quorum of recovery shares.**

---

## 3. Case (a) — the Google account is lost

### 3.1 What is actually lost

With claim binding (§4.3.1: the ID key signs "this key claims hosted account
`google:<sub-hash>`"), losing the Google account loses **one door**, not the identity:
the key file still signs, directory entries still verify, group membership is unchanged.
What breaks is the ability to *sign in to the hosted account* — i.e. to reach the DO that
stores the user's ciphertext, session metadata and share mailbox.

That asymmetry is the point of claim binding, and it is also why §4.3.2 must stay rejected:
if the server held the key, "lost Google account" and "lost identity" would collapse into
one incident with one recovery authority.

### 3.2 Recovery paths

1. **Key-native sign-in (preferred).** If the account is `AccountKeyed`
   (`directory.rs:155`), the client signs a challenge with the key and the directory
   resolves the account from the public key. No Google involvement; SSO is optional.
2. **Add a second Google identity before loss.** A second `sub` bound by a statement signed
   by the identity key. Losing one Google account leaves the other door.
3. **Rebind via a surviving key.** A signed `AccountKeyAdded`-style statement from any
   currently bound key naming the new `sub` hash. This works with no Google at all.
4. **Rebind via recovery quorum.** If no key survives, fall through to §6 (L2 shares): the
   quorum authorises a *new* identity plus a continuity statement (§5.4). Google plays no
   part.
5. **Email one-time code as a weak fallback** (`directory_auth.rs:26-29`, 15-minute codes).
   Acceptable for reaching an account whose keys are intact; **not** acceptable as authority
   to change a key binding, because email compromise is cheaper than Google compromise.

### 3.3 Threat: SSO takeover

An attacker who takes over the Google account reaches the door. Under these rules they get
the account's *server-side reachable* state — which, because content is encrypted to user
keys, is mostly metadata — and they cannot sign anything as the identity. They must not be
able to add a key. Two implementation notes: the rebind endpoint must require a signature
over a server-issued challenge including the new binding and a nonce, and the *old* keys
must be notified (all bound devices) with a delay window before a rebind takes effect.

---

## 4. Case (b) — the device or key is lost

### 4.1 Today: identity loss is total and silent

No export, no backup, no recovery (`§1.1`). A user who loses a laptop loses every group
they were a member of as far as their own participation is concerned; their past signed
objects remain valid and verifiable, but nobody can sign as them again, and no mechanism
exists to prove "the new key is the same person" except out-of-band human judgement
(admin-verified accounts, `directory.rs:189` `verify_account`).

This is *correct behaviour* in the sense the issue means: with the key destroyed and no
shares, the identity is gone. The design question is not whether to prevent that, but how
many independent ways out to offer before it happens.

### 4.2 L1 — passphrase-wrapped encrypted backup

The key is encrypted locally and stored as an opaque blob:

```
backup_ct = AEAD(KDF(passphrase, salt, Argon2id), identity_secret, aad = identity_pub || epoch)
```

Three placements, all server-blind: an exported file/QR the user keeps, the server's blob
store (ciphertext only, keyed by a hash of the identity public key), and optionally a
guardian. This is the single highest-value slice: it turns "lost device = identity gone"
into "lost device + passphrase = identity back". Costs: passphrase strength becomes the
weakest link (Argon2id parameters are the mitigation), and an attacker who steals the blob
gets an offline guessing target — mitigate with a high memory cost. The salt is
**not** a secret and must not be treated as one (corrected per Codex review,
PR #2278): the backup envelope's cleartext header carries the salt and the Argon2id
parameters beside the ciphertext, in every placement including the file-only
export. Separating the salt from the ciphertext would create a second required
recovery artifact with no storage or recovery path of its own — if the salt stayed
on the lost device, the passphrase plus the saved backup could not derive the
unwrap key and the L1 promise would fail. Splitting the salt off is only legitimate
as an explicit opt-in where the salt is independently backed up and listed as a
recovery prerequisite.

### 4.3 Post-loss without a backup: what can and cannot be rebuilt

- **Can**: membership in groups where the user is an admin or where an admin re-admits the
  new key; access to data still readable from another device; the account record, if a key
  survives (`AccountKeyAdded`).
- **Cannot**: decrypt past group epochs **for which no surviving member or backup retains
  the secrets**. A welcome is delivered to a member key (`encryption/welcome.rs`), not to an
  identity in the abstract — but loss of the only key that received the original welcome is
  not by itself the limit (narrowed per Codex review, PR #2278): a member that retains past
  epoch secrets can re-deliver them to the user's replacement key —
  `encryption/scheme.rs::add_with_history` transfers every retained secret with
  `HistoryMode::Full`, and `answer_history` can reseal older secrets to a current member;
  `test_joiner_gets_its_older_epochs_from_a_member` exercises exactly this recovery path.
  The hard, cryptographic limit applies only to epochs whose secrets no surviving member
  or backup retains.
- **Cannot**: re-sign anything. Past objects stay valid under the old key; no delegation
  helps retroactively.

### 4.4 Key rotation and continuity

The honest state: there is **no identity-succession primitive** in the codebase. What
exists is capability delegation (`world_session` `delegate_to_friend`, narrower scopes over
a session) and account-level key addition — neither is a statement "key B is the successor
of key A for identity X".

Two designs, in increasing order of work:

1. **Delegation certificate (buildable now).** `succession = Sign_A("successor:B,for:X,
   epoch:n,not_after:t")`, stored in the directory and in each group's log. A verifier that
   trusts A accepts B for X, with a chain back to A. Costs almost nothing; gives continuity
   of *authority*, not of decryption.
2. **Threshold identity (later).** The identity key is a threshold key; recovery with k-of-n
   shares produces a signature under the *same* public key, so no succession statement is
   needed and group membership is untouched. This is the clean answer, but nothing in
   `pkgs/id` does threshold signing today and the crypto dependency would be new work.

Recommendation: ship (1) as the continuity mechanism, keep the key format able to grow into
(2) by storing an algorithm/version tag beside the public key.

---

## 5. Case (c) — multi-device

### 5.1 What the code supports

`Account.keys` (`directory.rs:85`) is literally "further public keys that sign in as this
account", and `AccountKeyAdded` / `AccountKeyRemoved` (`:166-178`) are durable entries. So
the *account* layer already models N keys per identity. The *node* layer does not: each
device gets its own `.iroh-key` (or `.iroh-key-client`) in its own working directory, and
nothing binds those device keys to one identity.

Group encryption treats a member as a verifying key (`roster.rs:31`), so today the natural
reading is "each device is a member". That works but costs a rotation per device add/remove
in every group, and a device removal looks like a person leaving.

### 5.2 Recommended shape

- One **identity key** per person (ideally threshold, §4.4).
- One **device key** per device, with a device certificate
  `Sign_identity("device:D,not_after:t,scopes:…")`.
- The directory records device certs as account keys; group membership is *authorized*
  by identity key, while the encryption roster's recipients are the per-device keys
  (see the removal bullet below for why the roster cannot be identity-only).
- Adding a device: existing device (or identity key) signs the cert; the new device gets a
  welcome for each group so it can decrypt going forward, and epoch history only as far as
  the welcome carries (`encryption/welcome.rs`, `recent:N` bundles in `scheme.rs`).
- Removing a device: revoke the cert **and remove that device's own encryption
  recipient**, then rotate group keys once. Cert revocation alone does not exclude
  anyone from group content (corrected per Codex review, PR #2278):
  `AccountKeyRemoved` only deletes an account *sign-in* key, while
  `encryption/scheme.rs::Session::update` distributes the new epoch secret to every
  current roster member — with an identity-only roster there is no device-specific
  recipient to exclude, and a revoked device that retained the identity's encryption
  state keeps receiving new epochs. Exclusion therefore requires device-level
  recipients in the encryption roster: the roster entry removed on revocation is the
  *device* key, and the identity key's role is authorizing roster changes (via the
  device certs), not being the sole recipient. Per-device membership is retained at
  the encryption layer even though the account layer groups keys by identity. A
  device that was given a wrapped copy of the identity key itself (§5.3) cannot be
  excluded this way at all — it holds the identity, and only identity rotation (a
  new identity key plus succession, §4.4) cuts it off.

This keeps the "one person, many keys" fact in the account layer where it already lives.
Device churn no longer reads as a person joining or leaving a group; it is a recipient
change under one continuing identity membership — the price of making exclusion real.

### 5.3 Multi-device as recovery (the cheap win)

Two paired devices are already a 2-of-2 recovery scheme if the second device holds a wrapped
copy of the identity key rather than only its own device key. The pairing channel is the
existing invite channel (`invites/invitation.rs`, `qr.rs` for the scan, Seitan for the
proof) — the QR/link carries the invite, the accept proves key possession, and the payload
after acceptance is the wrapped identity blob. No server involvement beyond relaying
ciphertext.

---

## 6. Case (d) — social recovery

### 6.1 Mechanism: k-of-n shares of the unwrap key, not of the identity

Distribute Shamir shares of the **L1 unwrap key** (or of a random recovery secret that
itself wraps the identity key), k-of-n, to guardians:

```
share_i = SSS.split(recovery_secret, n, k)[i]
escrow_i = account_key_i_encrypt(share_i, aad = identity_pub || epoch || i)
```

**Prerequisite: the wrapped identity blob must survive independently of the shares
(Codex review, PR #2278).** This mechanism distributes shares of the unwrap key only —
not the wrapped identity ciphertext. If the user's only copy of the blob is the
file-only L1 backup permitted in §4.2 and that file is lost with the device,
reconstructing the unwrap key restores nothing: there is no ciphertext left to unwrap.
L2 setup therefore requires durable replication of the wrapped blob (server-stored
ciphertext, or copies held alongside the guardian shares) as a hard prerequisite.

Guardians are identified by their **account keys**, learned at an invite acceptance and
confirmed out of band by comparison of the **full account key** (the full hex verifying
key) over an authenticated channel — not by the petname fingerprint (corrected per
Codex review, PR #2278): `petname.rs:16`'s fingerprint is the first 6 hex characters
(24 bits) of the account ID, a contact-display disambiguator an attacker can match by
grinding keys, so it authenticates nothing and a substituted invitation could pass it
and receive a recovery share. Where a short form is needed, derive a safety number of
at least 128 bits from the full key; never reuse the display prefix as one. The
Seitan token key must not be reused as the guardian identity — it is single-use by design
(`seitan/mod.rs:6-9`). Note explicitly: today's Seitan is an invite proof, **not** a
safety-number comparison, so "SEITAN-verified contact" means "we completed an invite whose
accept was signed by a key the other side holds" — a channel, not a durable identity claim.

### 6.2 Flow

1. Recovery client proves possession of ≥k valid shares by decrypting and reassembling.
2. It reconstructs `recovery_secret`, unwraps the identity key (or threshold-signs directly,
   §4.4 option 2).
3. It publishes a signed recovery statement: new device cert, epoch bump, and a
   `not_before` delay (e.g. 72h) during which any currently bound device may veto.
4. All bound devices and guardians are notified; after the delay with no veto, the new
   device is live.

### 6.3 Threat: guardian collusion

k guardians who collude can reconstruct the secret and impersonate the user. Two
limitations of the obvious mitigations must be stated plainly (Codex review, PR #2278):

- **The veto window does not gate direct signing.** Step 2 of §6.2 hands colluding
  guardians the *existing* identity signing key immediately, before any delay begins.
  They can sign ordinary requests as that identity without publishing a recovery
  statement or registering a new device: `directory_auth.rs::verify_signed` verifies
  signatures without distinguishing a recovered copy of the key. The §6.2 veto therefore
  governs only the *registered* recovery path (a new device cert taking effect), not
  impersonation with the reconstructed key. Closing this hole requires a recovery
  authority that cannot exercise the existing identity's signing power before the veto
  completes — e.g. threshold signing (§4.4 option 2), where shares authorise a *new*
  key and the existing signing key is never reconstructed.
- **Epoch binding is not revocation.** Shares bound to `identity_pub || epoch` do not
  make a reconstruction useless after an epoch bump: former guardians who retain k old
  shares *and* the corresponding wrapped identity backup can still reconstruct the old
  unwrap key and decrypt that backup, and even a new unwrap key leaves the old backup
  able to reveal the same identity signing secret. Epoch binding is a freshness scope
  for shares, nothing more. A recovery (or rotation) that must defeat retained shares
  has to specify **replacement of the identity key and its revocation across
  verifiers**; until verifiers reject the old key, retained-share collusion works.

Mitigations that do hold, in order of value: (i) the veto window lets the user's
surviving devices cancel a hostile *registered* recovery; (ii) guardians hold
ciphertext they cannot read alone, so k-1 guardians learn nothing; (iii) choose k
relative to n so that collusion requires socially implausible coordination (3-of-5
rather than 2-of-3 for high-value identities).

Residual risk is social, not cryptographic: a guardian's device compromise plus k-1 other
compromises is the attack, and no protocol removes it.

---

## 7. Threat analysis, per path

| Path | Server collusion | Guardian collusion | SSO takeover | Replay of recovery proof | Other |
| --- | --- | --- | --- | --- | --- |
| L0 device key only | Server never sees it; nothing to collude over | n/a | Cannot sign; can only reach server-side metadata | n/a | Loss = identity loss (by design) |
| L1 wrapped backup, server-stored | Server holds ciphertext; must break Argon2id + AEAD | n/a | Blob is useless without passphrase | Blob is static; replay is not meaningful, theft is | Offline guessing; passphrase reuse |
| L1 file/QR backup | No server exposure at all | n/a | No | Same as above | User loses the file too; QR shoulder-surfing |
| L2 Shamir among guardians | Server relays escrow blobs only | **Yes — k colluding guardians = takeover**; bound by identity-key revocation + veto window (epoch binding alone is not a mitigation, §6.3) | Cannot mint shares | Recovery statement must be single-use and persisted, not the in-memory 5-min cache (`directory_auth.rs:39`) | Guardian churn requires resharing; dead guardians reduce n |
| L3 SSO as account pointer | Operator can read metadata and could *try* to rebind | n/a | **Gets the door, not the key** — provided rebind requires a key signature | Challenge must bind nonce + new binding + expiry | Phishing the Google login; `sub` recycling is excluded by Google's spec but hash the `sub` anyway |
| Delegation certificate | Server stores a signed statement; cannot forge it | n/a | Cannot produce a successor statement | Certificate has `not_after` and epoch | Old key must still exist to sign — useless after total loss |

Two rules fall out and belong in the implementation, not just the doc:

1. **Recovery proofs are persisted, single-use, and bound to a recovery session**, with a
   minimum validity of hours rather than `SIGNED_WINDOW_MS`. `directory_auth.rs`'s
   in-memory nonce cache (`SIGNED_NONCE_LIMIT = 4096`) is fine for logins and wrong for
   identity changes.
2. **Any identity change is announced to every bound device before it takes effect.** The
   veto window is the only thing standing between a guardian quorum and a silent takeover.

---

## 8. Recommended recovery model for the hosted edition

### 8.1 The layers

| Layer | Holds | Server sees | Recovers | Fails when |
| --- | --- | --- | --- | --- |
| **L0** device key | Identity key (or device key + cert) in `.iroh-key`, `0600` | Nothing | Nothing — this is the thing to protect | Device lost/stolen |
| **L1** wrapped backup | AEAD(Argon2id(passphrase), identity key) | Ciphertext blob only | Lost device, with passphrase | Passphrase forgotten *and* no L2 |
| **L2** social shares | k-of-n shares of the unwrap key, encrypted to guardian account keys | Ciphertext escrow only | Lost device and lost passphrase when k guardians respond — and a lost backup **only if the wrapped blob was durably replicated** (§6.1 prerequisite); shares alone cannot restore a lost blob | Fewer than k guardians reachable; k collude |
| **L3** SSO | `google:<sub>` hash bound to the account by a key-signed claim | The `sub` hash and the binding record | Account *access* only — never a key | Google account lost (falls back to L0/L1/L2) |

### 8.2 The flows

```mermaid
flowchart TD
    subgraph Normal["Normal use"]
        D1[Device 1<br/>identity key L0] -->|signs device cert| D2[Device 2<br/>device key]
        D1 -->|wrap with passphrase| B1[(L1 backup blob<br/>ciphertext)]
        D1 -->|Shamir k-of-n| G[Guardians<br/>L2 escrow shares]
        D1 -->|signed claim| S[(Directory<br/>account + google sub hash)]
    end

    subgraph LostGoogle["(a) Google account lost"]
        LG[SSO fails] --> K1{Any bound key<br/>still available?}
        K1 -->|yes| K2[Sign challenge with key<br/>add or replace SSO door]
        K1 -->|no| K3[Fall through to (b)]
    end

    subgraph LostDevice["(b) Device or key lost"]
        LD[Key gone] --> P1{Passphrase<br/>+ L1 blob?}
        P1 -->|yes| P2[Argon2id unwrap<br/>identity restored]
        P1 -->|no| P3{k guardians<br/>respond?}
        P3 -->|yes| P4[Reassemble secret<br/>unwrap identity from<br/>replicated blob (§6.1)]
        P3 -->|no| P5[Identity lost<br/>correct outcome]
        P2 --> R[Recovery statement<br/>new device cert + epoch]
        P4 --> R
        R --> V{72h veto window<br/>any surviving device objects?}
        V -->|no| LIVE[New device live]
        V -->|yes| ABORT[Abort, notify all]
        LIVE --> CHAIN[Delegation certificate<br/>old key to new key, if old key survived]
    end

    subgraph Multi["(c) Multi-device"]
        MD[Add device] --> MDC[Existing device signs cert]
        MDC --> WEL[Welcome per group<br/>decryption from here on]
        WEL --> DIR[Directory records the device key]
    end

    S -.->|Never rebinds a key| D1
```

### 8.3 Why this order

L1 is the cheapest change with the largest effect and no new cryptography beyond a KDF and
an AEAD. L2 is where most of the social design lives and it is worthless without L1's
unwrap structure, so it follows. L3 is a rule about a login flow that does not exist yet,
so it costs nothing to specify now and is expensive to retrofit later — which is exactly
why it is written down here.

### 8.4 Explicitly rejected

- **Custodial keys (§4.3 option 2).** Breaks LIST 7; makes "lost Google account" and
  "lost identity" one incident with one recovery authority. Rejected.
- **SSO-driven rebind.** Even with a valid Google login, a rebind requires a signature from
  an already-bound key or a share quorum. Rejected.
- **Server-side escrow of the unwrap key "just in case".** Same failure as custodial with
  extra steps.
- **Reusing the Seitan invite key as a guardian identity.** Single-use (`seitan/mod.rs:6-9`).

### 8.5 Cases that remain unsolved, and why

1. **Key destroyed on all devices, no backup, fewer than k shares** — identity is gone.
   Correct: the whole point of the key being the identity is that nobody can conjure it.
   The product answer is a loud, repeated backup prompt, not a recovery path.
2. **Past group epochs whose secrets no surviving member or backup retains** — unrecoverable
   even after successful identity recovery. (Epochs a surviving member retains are
   recoverable by history re-delivery, §4.3.) Cryptographic, not fixable by policy;
   mitigation is redundant device membership and `recent:N` welcome breadth choices.
3. **k guardians collude while the user has no surviving device** — takeover succeeds. The
   veto window only helps if some device survives; with none, the notify step protects
   nobody. Social risk, not solvable in protocol.
4. **Threshold signing / no-successor-needed rotation** — not implemented; requires a new
   crypto dependency. Until then, recovery after total key loss produces a *new* key with a
   delegation chain that can only be signed if something of the old authority survived.
5. **Guardian liveness over years** — shares die with devices, people stop answering, and
   resharing needs a live identity. No proactive-reshare design is proposed here.
6. **Bearer-backed accounts are server-signable** — `sign_up` (`directory.rs:561`)
   returns `acct.<id>.<secret>` after the server generated it, and the signing key is
   derived from that secret (`signing_key`, `directory.rs:1769`), so `authenticate`
   (`directory.rs:1432-1457`) reconstructs it on every presentation and the publish /
   friend-request paths sign with it. Digest-only storage therefore does not make the
   server unable to sign — only unable to sign *without the secret*, which it
   generated and saw. Anyone retaining the secret retains signing power.
   `AccountKeyed` avoids this entirely and should be the hosted default.
7. **Replay-hardening of recovery proofs** — needs persisted single-use records; today's
   machinery is an in-memory 4096-entry cache that a restart forgets
   (`directory_auth.rs:39`).

---

## 9. Risks of this recommendation

- **Passphrase fatigue.** L1 shifts the risk from "no recovery" to "weak passphrase". Without
  Argon2id at a serious memory cost, a stolen blob is an offline attack.
- **Guardian UX is the hard part.** Choosing k-of-n, explaining that guardians hold
  ciphertext and cannot read it, and handling resharing when someone disappears is more
  product work than protocol work.
- **Device certs add a layer that does not exist yet.** The directory models multiple keys,
  but nothing today binds a device key to an identity key; this is new code, and it must
  not become a second, weaker identity system.
- **Veto windows conflict with "instant recovery".** Users will ask why recovery takes 72
  hours; the answer is that instant recovery is instant takeover.
- **Specifying L3 before SSO exists** risks the spec drifting from the eventual OIDC
  implementation; the rule ("SSO never rebinds") is stable, the exact claim format is not.

---

## 10. Open questions for the design phase

1. Is the identity key ever *exported* in the hosted edition, or only ever unwrapped in
   memory on a device? Export changes the threat model for malware on the device.
2. Do device certs live in the directory (visible, revocable) or in a user-private document
   (invisible to the operator)? Both are consistent with LIST 7; the first is auditable, the
   second leaks less.
3. Should the L1 blob be stored server-side by default, or opt-in with a local-file default?
   Default-on recovers more users; default-off leaks less.
4. What is the minimum viable k-of-n, and does the product cap n to keep the UX honest?
5. Can a recovered identity rejoin groups without an admin, using the delegation chain? This
   needs a group-level rule that does not exist today.
6. Does the hosted edition's account creation use `AccountKeyed` exclusively, retiring
   `AccountSignedUp` for new users? If yes, the "server saw the secret" risk in §8.5.6
   disappears for everyone who joins after the change.

---

## 11. Follow-on slices (proposed, for the synthesis to own)

These are **not filed**. The gated synthesis issue that owns #2060's outcome should decide
which of them become issues, in which order, and against which milestone.

1. **`id key export` / `id key import`** — a documented, passphrase-wrapped export of the
   identity key, with Argon2id parameters in the format header. Files: `store.rs`,
   `commands/id.rs`. Prerequisite for everything else.
2. **L1 backup blob format and storage** — AEAD + KDF versioned envelope, ciphertext blob
   keyed by identity-public-key hash; server stores bytes only. Depends on (1).
3. **Recovery statement and persisted replay protection** — single-use, hours-long recovery
   sessions with a veto window, replacing the in-memory nonce cache for identity changes.
4. **Device certificates** — `Sign_identity(device key, not_after, scopes)` in the directory,
   reusing `AccountKeyAdded`; define the acceptance rule (who may add, what proof).
5. **Delegation certificate for identity succession** — `Sign_A(successor:B, epoch, not_after)`
   verified back to a trusted root; needed so a recovered identity keeps authority.
6. **Shamir share distribution over the invite channel** — escrow format, guardian binding to
   account keys, resharing rule; the QR/invite machinery (`invites/`, `qr.rs`) already exists.
7. **SSO binding record and the "SSO never rebinds" rule** — signed claim format, hashed
   `sub`, rebind endpoint that requires a key signature or a share quorum, notification to
   bound devices.
8. **`AccountKeyed` as the hosted default** — turn off server-generated bearer credentials
   for new hosted accounts; document the migration for existing ones.

## Sources

- `doc/2026-10-28T23-34-14Z_investigation_cloudflare_workers_fit/` §4.3 (claim binding vs
  custodial vs browser-held key) and §8 (this question).
- `pkgs/id/src/lib.rs:292,298` — `KEY_FILE`, `CLIENT_KEY_FILE`.
- `pkgs/id/src/store.rs:58-190` — keypair creation, raw 32-byte format, `0600` permissions.
- `pkgs/id/src/directory.rs:78-93` — `Account`, including `keys` and `emails`.
- `pkgs/id/src/directory.rs:140-180` — `AccountSignedUp`, `AccountKeyed`, `AccountKeyAdded`,
  `AccountKeyRemoved`.
- `pkgs/id/src/directory.rs:556-580` — `sign_up`, digest-only storage, one-time credential.
- `pkgs/id/src/directory_auth.rs:24-39, 60-90, 314-350` — sessions, credentials, signed
  requests, 5-minute window, in-memory replay cache.
- `pkgs/id/src/seitan/` — Keybase Seitan V2 invite tokens (`mod.rs:1-17`, `token.rs:10`,
  `derive.rs:3-9`, `admit.rs`, `freshness.rs`): an invite proof, not safety numbers.
- `pkgs/id/src/invites/invitation.rs:17-25` — `id:join?` link with the inviter's account key.
- `pkgs/id/src/encryption/roster.rs:31`, `scheme.rs`, `welcome.rs` — member = verifying key,
  welcome bundles, epoch rotation.
- `pkgs/id/src/web/identity.rs:131,483-620` — server-derived keys protecting the server's own
  collab records; not a precedent for user key custody.
- `pkgs/id/src/petname.rs:16` — local-only names with a 6-character fingerprint.

Repo facts stated as unverified or partial: whether `AccountKeyAdded` admission currently
requires the added key's own signature (the entry carries only the public key; the applier's
rule was not traced), and whether any hosted SSO deployment exists outside this repo. No
Google/OIDC/passkey code was found in `pkgs/id/src`, so all SSO content here is specification.
