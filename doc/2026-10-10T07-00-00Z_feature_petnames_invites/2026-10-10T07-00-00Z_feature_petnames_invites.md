# Petnames and invite links

## Intent

Accounts in `id` are self-certifying: the account ID is the hex Ed25519 public
key, and the display name is alleged, not verified. A name a server claims is
not enough to tell two people apart, and a stranger can register any name,
including one that looks like a contact's. This change lets a signed-in account
give its own private name to another account (a petname, after Stiegler), and
gives people a way to hand each other an invite that carries a key and a name.

## Plan

### Petname store

- A per-signed-in-account store in `petnames.jsonl` in the world's data
  directory, opened in `world_store::open_world` beside `directory.jsonl`.
- Local only: it is never replicated, never sent in envelopes, and never
  written to the directory journal.
- Append-only log of `set` and `remove` lines, replayed on open, the same
  pattern as `directory.jsonl`.
- Both-direction uniqueness per owner: one name maps to at most one account,
  and one account has at most one name. Comparison folds case, so `Bo` and `bo`
  cannot both be used.
- Only the owner may set, rename or remove a petname. Rename requires an
  existing petname; set refuses a name already in use.

### Mimicry check

On create or edit, a candidate name is compared against the alleged names of
every other account, and against the owner's other petnames. Two comparisons:

- Case-folded equality.
- Confusable-folded equality: lowercase, then map look-alikes (`0`→`o`,
  `1`/`i`/`|`→`l`, `3`→`e`, `5`→`s`, `$`→`s`, `@`→`a`, `rn`→`m`, `vv`→`w`).

A match on either is refused. Refusal, not a warning, is the policy: a warning
is easy to click through, and the point of the check is to stop the name from
being used at all. The target account's own alleged name is excluded, so an
account may be given its own name back.

### Display rule

A pure function over the owner's contacts:

- A contact with a petname shows the petname.
- A contact without one shows its alleged name in a distinct style, marked
  `unverified`, with a short fingerprint (the first 6 hex characters of the
  account ID).
- Two contacts whose petnames collide after folding both show the petname with
  their 6-hex fingerprint suffix.

### Invite URL

- Form: `id:invite?key=<hex>&node=<hex>&name=<url-encoded>`.
- `key` is the account ID (64 hex), `node` is the server's node ID (64 hex),
  and `name` is the alleged name, percent-encoded. Encoding and decoding are
  hand-written, and `+` decodes to a space.
- The `id:` scheme is only a string format. Nothing registers it with an
  operating system, and nothing resolves it. It is a text value that people
  copy and paste.
- The HTTPS form is `GET /invite?key=…&node=…&name=…`. It shows an "Add
  <alleged name>" confirmation. Confirming goes through the explorer's
  existing sign-in, and sets a petname for the key.

### QR

- The invite page carries an SVG QR code of the `id:invite?…` string.
- The SVG is generated with the `qrcode` crate's encoder, as module paths
  written by this code, so the test can parse the SVG back, rasterize it, and
  decode it with `rqrr` (dev-dependency) to check the same string comes back.

### Explorer

- Grammar: `petname set ACCOUNT NAME`, `petname rename ACCOUNT NAME`,
  `petname remove ACCOUNT`.
- The same three as HTML form actions, in `ACCOUNT_FORMS`.
- The contact list shows the viewer's own petnames. Admin and anonymous
  viewers never see petnames.

## Decisions

- Store location: `petnames.jsonl` beside `directory.jsonl`, not in the
  directory journal, so petnames are never replicated.
- Mimicry: refuse, for both exact (folded) and confusable matches.
- Fingerprint: first 6 hex characters of the account ID, shown only on
  collision or for unverified alleged names.
- Invite `key` is the account ID, so a confirmed invite sets a petname for
  exactly the account that issued it.
- `doc/` at the repository root holds this document because the lead asked for
  it there. `pkgs/id/docs/` is where the earlier id designs live.

## Implementation: petnames and invite links

- `src/petname.rs` (feature `world`): the `Petnames` store, its normalization,
  folding and confusable skeleton rules, and the `labels` display rule.
- `src/invite.rs` (ungated): `Invite` values, the `id:invite?…` and
  `/invite?…` forms, and hand-written percent encoding.
- `src/qr.rs` (feature `web`): QR code SVG through `qrcode` 0.14.1.
- `src/directory_view.rs`: `SetPetname`, `RenamePetname` and `RemovePetname`
  actions, the SSH text grammar and the HTML form fields. `render_text` now
  shows each account with its label.
- `src/web/explore.rs`: `/invite` confirmation page, petname forms, and the
  italic style for unverified alleged names.
- `src/world.rs`, `src/world_store.rs`: `WorldCore` holds the petname store,
  opened from `petnames.jsonl` beside `directory.jsonl`.

Behavior change: `render_text` marks every account that has no petname as
"(unverified fp)". Existing text tests still pass.

Not done: an end-to-end HTTP test of the `/explore/act` petname flow. Clippy
could not be run because of a rustc mismatch in this environment (see the
commit report); tests and `cargo fmt --check` pass.
