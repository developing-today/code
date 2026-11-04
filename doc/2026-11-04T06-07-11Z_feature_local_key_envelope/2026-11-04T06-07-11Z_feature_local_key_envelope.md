# Local passphrase-wrapped identity key envelope

**Date:** 2026-11-04 · **Issue:** [#2279](https://github.com/developing-today/code/issues/2279)

## Intent

Add a local-only way to move the existing 32-byte ID identity key to another
device without sending the key or passphrase to a server. This is only the
first recovery slice; seed phrases, server backups, recovery sessions, key
rotation, guardians and hosted-object transfer remain separate work.

## Format and security decisions

- Version 1 is exactly 108 bytes: 60-byte header (`IDKEY` magic [5 bytes],
  version [u8], KDF id [u8], cipher id [u8], memory/time/parallelism [three
  little-endian u32s], salt [16 bytes], nonce [24 bytes]) followed by the
  32-byte key ciphertext and 16-byte Poly1305 tag. KDF id 1 is Argon2id v=19;
  cipher id 1 is XChaCha20-Poly1305. The complete header is AEAD associated
  data, so parameter or version changes are authenticated.
- Derive the 32-byte AEAD key with Argon2id v=19. Version 1's accepted floor is
  19,456 KiB memory, 2 iterations and 1 lane; the writer uses exactly that
  floor. The reader also rejects values above 262,144 KiB, 10 iterations or 4
  lanes before running the KDF, limiting malicious-input resource use.
- Reject unknown versions/algorithms, malformed/truncated input and parameters
  outside the supported range. Authenticate before returning any plaintext.
- Prompt for passphrases without terminal echo; never accept them as command-line
  arguments, and do not log key material or passphrases. File import remains
  local and refuses to replace either identity key unless explicitly requested.
- Write exported envelopes and imported key files with owner-only permissions;
  create imports without clobbering by default and replace atomically only with
  `--replace`.

## Initial implementation plan

1. Add the envelope codec and focused tests for round-trip, wrong passphrase,
   truncation/tampering and below-floor parameters.
2. Add `id key export` and `id key import`, preserving the separate server and
   client identities and using the existing raw-key storage format on import.
3. Document the command and format, run the focused Rust tests and CLI checks.
