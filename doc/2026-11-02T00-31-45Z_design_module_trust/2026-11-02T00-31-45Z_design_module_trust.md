# Design: module trust — admin trust list and install-time signature check

Issue: developing-today/code#1069 (F12.17 trust and provenance). Implemented
in PR #4088 (task 4945). This record was added during Codex review of that
PR, per the pkgs/id documentation protocol; it describes the design as
implemented and its intentional limitations.

## Problem

Worlds run wasm/native modules installed over the session protocol by
whoever holds the world admin token. Observed provenance (#225) records
what is running, but nothing decided whether an artifact *should* run:
there was no admin trust list for module signing keys and no signature
verification at install time.

## Design

- **Trust list.** The server operator configures Ed25519 public keys with
  the repeatable `serve --world-module-trust-key <hex>` flag. Keys are
  parsed at startup (32-byte hex; anything else is a startup error) and
  carried into every `WorldService` the hub opens
  (`WorldService::with_module_trust_keys`).
- **Signing format.** A signature covers a domain-separated digest:
  `b"developing-today/id-world-module/v1\0"` followed by the 32-byte
  BLAKE3 digest of the exact module bytes, signed with Ed25519
  (`verify_strict`). The upload carries it as an optional `signature`
  object (`ModuleSignature { public_key, signature }`, hex) on the
  `install_begin` frame; the field is part of the shell contract
  (`contract/shell-contract.v1.json`).
- **Gate.** Verification runs in `install_wasm_reserved`, after the
  claimed-hash check and before native/wasm validation:
  - empty trust list → no check; installs behave exactly as before;
  - non-empty list → the artifact must carry a signature from a listed
    key; unsigned, unlisted-key, and bad-signature uploads are refused
    with `InstallError::UntrustedModule` ("module signature is not
    trusted" on the session surface).

## Policy interactions (intentional)

- **Server-compiled modules are exempt.** `compile_and_install` artifacts
  are produced by the server's own Roc compiler from source and never
  cross the upload path (`compiled_from_source`); the gate does not apply,
  so `--world-source-only` program updates keep working with a trust
  list configured.
- **Startup modules are not yet signable.** `--world-module` installs
  through the unsigned path, so combining it with
  `--world-module-trust-key` is rejected at startup with an explicit
  error rather than failing obscurely at world-open time. A
  signature-aware startup path is future work.
- **Admin token still required.** The trust gate is additive: it does not
  replace admin authorization for installs.

## Limitations / follow-ups (still open on #1069)

- Compiler version is not yet recorded in observed module provenance.
- Trust decisions (accept/refuse + key + module identity) are not yet
  recorded in the journal; today they surface only as install results.
- No key rotation/revocation story beyond editing the flag at restart.
