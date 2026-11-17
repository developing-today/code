# 1Password CLI (`op`) — Setup Cycle

Verified 2026-10-21: `op` 2.41.0 at `~/tools/1password-cli/op`, symlinked from `~/tools/bin/op`.
Status honesty note: the binary is installed and runs, but TOOLS.md records **no completed sign-in / vault workflow** for `op` on this sandbox. The primary secrets system here is Bitwarden Secrets Manager (see bitwarden-bws.md). Treat everything below the Install step as "binary present, account flow not yet documented as working" until a test is actually run and recorded.

## Download
- Binary + signature file present as a pair: `~/tools/1password-cli/op` and `~/tools/1password-cli/op.sig` (the `.sig` is 1Password's published signature for verifying the CLI binary). Upstream source: https://downloads.1password.com/op/ (UNSOURCED in TOOLS.md — the exact download URL/version pin used for this install was not recorded; verify against 1Password's official CLI install page before re-downloading).

## Install
- Place the verified binary at `~/tools/1password-cli/op` (persistent disk) and symlink it:
  ```bash
  ln -s ~/tools/1password-cli/op ~/tools/bin/op
  ```
- No package manager, no service. `~/tools/bin` is on PATH via `~/tools/shell-env.sh`.

## Gather secrets
- `op` authenticates against a 1Password account — that sign-in is interactive / account-based, not a file token like bws.
- No Bitwarden item name for a 1Password credential is recorded in TOOLS.md. **UNSOURCED:** whether a service-account token (`OP_SERVICE_ACCOUNT_TOKEN`) or Connect setup exists for this sandbox — none is documented, so assume interactive sign-in only until proven otherwise.
- Do NOT store an `op` credential inside Bitwarden docs or vice-versa by value — names only, per the library rule.

## Configure
- `op --help` itself says: "Turn on the 1Password app integration and sign in to get started. Run `op signin --help` to learn more."
- No `~/.config/op` configuration is documented in TOOLS.md for this sandbox.

## Test
```bash
op --version   # expected: 2.41.0  ✅ verified 2026-10-21
op --help      # expected: command list (account, connect, document, events-api, group, ...)  ✅ verified 2026-10-21
op account list  # NOT yet verified on this sandbox — this is the first real sign-in test to run and record
```

## Known quirks & what has NOT worked
- Nothing has been recorded as failed — because nothing beyond `--version`/`--help` has been recorded as *tried*. That gap is the finding: if a future workflow depends on `op`, run and document the sign-in test above first, and do not assume the Bitwarden flows transfer.
