# sign_request nonce domain — fallible signing contract (2026-11-08)

Issue: developing-today/code#6443 · PR: developing-today/code#6539

## Problem

`verify_signed` refuses any signed request whose nonce is empty, longer
than 64 characters, or contains a character other than an ASCII letter,
digit or '-'. `sign_request` used to sign any nonce unconditionally, so a
client could produce a well-formed signature that verification would
always reject as "signature does not verify" — a failure indistinguishable
from tampering, caused entirely by the signer's own output.

## Change

`sign_request` in `pkgs/id/src/directory_auth.rs` now validates the nonce
against the same domain before signing and returns
`Result<Signed>` instead of `Signed`:

- Allowed nonce domain: 1–64 characters, each an ASCII letter, digit
  or '-'. This is exactly the domain `verify_signed` accepts; the check
  lives in one helper (`nonce_in_domain`) used by both sides, so signer
  and verifier cannot drift apart.
- Rejection behaviour: an out-of-domain nonce fails with an error naming
  the nonce domain requirement; no signature is produced.
- Caller effect: this is a breaking change to a public API. Callers must
  handle the `Err` (in-repo callers were updated to `.unwrap()` only in
  tests, where the nonces are fixed in-domain literals). A caller passing
  a generated nonce should generate inside the domain (for example hex
  or a UUID) rather than recover from the error after the fact.

## Tests

- Example tests cover the boundary and bad-character cases (empty,
  65 characters, space, underscore, plus, dot, non-ASCII; valid: short,
  mixed-case with '-', exactly 64 characters) and round-trip the valid
  ones through `verify_signed`.
- A `proptest` property checks, for generated nonces, that
  `sign_request` succeeds exactly when an independent oracle predicate
  for the domain accepts, and that accepted nonces verify.
