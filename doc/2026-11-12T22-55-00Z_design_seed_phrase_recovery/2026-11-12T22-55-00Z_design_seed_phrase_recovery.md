# Seed-phrase export/restore for the identity key

**Date:** 2026-11-12 · **Issue:** [#2381](https://github.com/developing-today/code/issues/2381) · **Implementation:** PR #9036 (`pkgs/id/src/seed_phrase.rs`, `id key export-seed` / `id key import-seed`)

## Intent

Give the identity a human-transcribable backup: export the identity key as a
24-word seed phrase, and restore the identical identity from that phrase on
another machine, with zero server involvement. This is the seed-phrase slice
of #2381 (the encrypted identity backup umbrella) and doubles as the
hosted-edition exit ramp: a user who holds their phrase never needs the
hosted service to recover their identity. The passphrase-envelope slice
(#2279, `key_envelope.rs`) already landed; the server-side backup blob /
OPRF slice and the restore-forces-rotation flag remain separate follow-ons
on #2381.

## Format decision: direct key-byte encoding, not BIP39 seed derivation

The phrase encodes the key's 32 bytes **directly**, using BIP39's encoding
layout over the official English wordlist (embedded verbatim in
`seed_wordlist.txt`):

- 256 bits of entropy (the key bytes themselves) plus the first 8 bits of
  their SHA-256 as checksum — 264 bits, split into 24 words of 11 bits.
- The encoder also accepts the other BIP39 entropy lengths (16/20/24/28
  bytes → 12/15/18/21 words) so the codec is a faithful BIP39 encoding;
  `encode_key`/`decode_key` restrict the identity path to 32 bytes / 24
  words. Published BIP39 test vectors pin the encoding.

This is deliberately **not** BIP39 seed derivation. In BIP39 proper, the
phrase (plus an optional passphrase) is run through PBKDF2-HMAC-SHA512 to
produce a 512-bit seed from which keys are then derived. Here there is no
PBKDF2, no passphrase, and no derived seed: the identity key already exists
as fixed bytes (the stored ed25519 keypair), and a restore must reproduce
**those exact bytes**, not deterministically regenerate a different key.
Compatibility is with BIP39's wordlist, bit layout, and checksum only —
enough that standard tooling and user familiarity transfer, and the phrase
can be verified against the published vectors — while the semantics stay
"the phrase **is** the key, encoded".

Consequence, stated plainly in the export output: anyone holding the phrase
holds the key. The phrase is a bearer backup of the secret itself, not a
hint toward it and not a share of it.

## Export/restore handling contract

- **Export** (`id key export-seed [--client]`): reads the existing identity
  key file (server or client identity per `--client`), encodes it, and
  prints the phrase **once**, with the handling guidance: write it down,
  keep it offline, anyone with it can restore and impersonate this
  identity. The phrase is not stored, logged, or written to any file by
  the command.
- **Restore** (`id key import-seed [--client] [--replace]`): prompts for
  the phrase without terminal echo (`rpassword`), never as a command-line
  argument. Decoding is strict because a restore must never silently
  produce a different key:
  - word count must be a BIP39 length and, for a key, exactly 24 words;
  - every word must be in the wordlist (input is normalized the way a
    person pastes: whitespace runs collapse, case folds);
  - the SHA-256 checksum is verified **before** any entropy byte is
    trusted — a wrong or out-of-order word is an error, never a
    best-effort decode.
- The restored bytes are written to the identity key file in the existing
  raw-key storage format, owner-only, created without clobbering: an
  existing key is replaced only with an explicit `--replace`, atomically.
  Server and client identities stay separate files, exactly as with the
  envelope import.
- **No server involvement at any step**: no upload, no account, no
  recovery session. That is what makes the phrase an exit ramp rather
  than another hosted dependency.

## Relationship to the #2279 envelope

The passphrase envelope and the seed phrase protect the same 32 bytes for
different carriers: the envelope is a passphrase-wrapped **file** for
device-to-device transfer; the phrase is a **human-transcribable** backup
for paper. They share the key and the storage format on import, and
deliberately do not share a format — this slice builds no second envelope,
per the task scope.

## Implementation and verification

1. `seed_phrase.rs`: `encode`/`decode` over the embedded wordlist, with
   `encode_key`/`decode_key` fixing the 32-byte identity case; decoded
   material is held in `Zeroizing` buffers.
2. CLI wiring in `commands/key.rs` (`export_seed` / `import_seed`) under
   `key export-seed` / `key import-seed`, reusing the key-file writers of
   the envelope commands.
3. Tests: published BIP39 vectors, wordlist integrity (2048 words, unique
   4-letter prefixes), round-trip, case/whitespace normalization, checksum
   / unknown-word / bad-count rejection — plus property tests naming their
   oracles (generated entropy round-trips at every BIP39 length; generated
   keys round-trip; a checksum-bit flip is always rejected; an unknown
   word is always rejected; non-BIP39 counts are always rejected; a valid
   sub-32-byte phrase decodes as entropy but is never accepted as a key).
