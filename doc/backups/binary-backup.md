# Binary backup manifest

Generated 2026-10-21. Drew's rule (2026-10-21): the toolchain **backup set includes the binaries themselves**, not just docs and secrets — including the Bitwarden CLI (`bws`) in addition to the Bitwarden secrets it guards.

Most entries in `~/tools/bin` are symlinks/dispatcher stubs that resolve reproducibly from nix/omnibin — those are covered by the setup-cycle docs and do not need byte-level backup. The standalone real files below are the ones to copy as-is.

| file | bytes | sha256 (first 16) | note |
|---|---|---|---|
| ~/tools/bin/age-keygen | 266 | 6b060af7d8c038e8 |  |
| ~/tools/bin/bws | 12359704 | e6982cfac1094d9b | Bitwarden Secrets Manager CLI |
| ~/tools/bin/cargo | 256 | a100950361d070f0 |  |
| ~/tools/bin/codedb | 4427904 | af44d5f02a6c9a28 |  |
| ~/tools/bin/cowsay | 258 | b7740b9809c9e427 |  |
| ~/tools/bin/crane | 256 | be91577b8a1ac8e4 |  |
| ~/tools/bin/deno | 254 | 4c7ada24e696fbdc |  |
| ~/tools/bin/docker | 258 | e3681a0254f0d1b0 |  |
| ~/tools/bin/fff | 252 | 3e167cda916b4f64 |  |
| ~/tools/bin/fff-mcp | 13355488 | 20917f2c3e35612d |  |
| ~/tools/bin/figlet | 258 | fe9c3942d930a249 |  |
| ~/tools/bin/fish | 254 | 3d3b597485d150b3 |  |
| ~/tools/bin/getfacl | 260 | c0968976ce93d357 |  |
| ~/tools/bin/git | 252 | 049121e426d9b97b |  |
| ~/tools/bin/go | 250 | 2c122df9bf24305a |  |
| ~/tools/bin/gofmt | 256 | 9d8bfd8a1df15613 |  |
| ~/tools/bin/hello | 256 | fe940ebd1ed3d9c5 |  |
| ~/tools/bin/just | 254 | e4361fbe2890472f |  |
| ~/tools/bin/nix-instantiate | 276 | 6e57018a7f535e4c |  |
| ~/tools/bin/nix-store | 264 | dd65b1979f6c21f5 |  |
| ~/tools/bin/oci | 252 | 380c4e135c86215f |  |
| ~/tools/bin/or | 250 | 4b85ec244ca08ff3 |  |
| ~/tools/bin/pg_dump | 260 | 37f2bbf8e11f0c9c |  |
| ~/tools/bin/prettier | 262 | c9b4365c02c6f163 |  |
| ~/tools/bin/profile | 260 | 78ff579ea1239248 |  |
| ~/tools/bin/ruff | 254 | c3faa7cc358d6ef5 |  |
| ~/tools/bin/rustc | 256 | 08c36859e3a9f1c6 |  |
| ~/tools/bin/rustfmt | 260 | a4c0e7a9f12e7977 |  |
| ~/tools/bin/rustup | 258 | e0753235a8cd1e4a |  |
| ~/tools/bin/screen | 258 | f1f614f1cef5fe46 |  |
| ~/tools/bin/shellcheck | 266 | 7a232ee2cc8806c4 |  |
| ~/tools/bin/shfmt | 256 | e6d81acf6578b9d5 |  |
| ~/tools/bin/skopeo | 258 | 1c0c1faae31fd5aa |  |
| ~/tools/bin/sops | 254 | e87db8469016699d |  |
| ~/tools/bin/supabase | 262 | 91af8ed5796cbf3e |  |
| ~/tools/bin/taplo | 256 | 9c5fa27d54713afc |  |
| ~/tools/bin/timeline | 262 | 193bbde90014c5a9 |  |
| ~/tools/bin/turso | 256 | 24ad55d62ff4cff2 |  |
| ~/tools/bin/uv | 250 | 060a1679f1359bbc |  |
| ~/tools/bin/xxd | 252 | 96ccfd733eff6a9b |  |

Toolchains installed as trees (back up the tree or reinstall via its doc): Go (~/tools/go), Rust (~/tools/cargo + ~/tools/rustup), wrangler (~/tools/wrangler), nix (persistent store — see nix.md, snapshot via ~/tools/nix-snapshot.sh).
Chromium + Playwright: install targets only, optional/headless — see chromium-playwright.md. Tailscale CLI: keep installed — see tailscale.md.

## Turso CLI provenance (resolved 2026-10-21)

The turso binary originally 'just existed' because it was a dispatcher/omnibin stub (Nix build of turso-cli 1.0.30), auto-created on first invocation — never a deliberate install. It has since been replaced with the official install (`get.tur.so/install.sh`, archived in `archive/install-scripts/`): real binary at `~/.turso/turso` (**v1.0.34**), `~/tools/bin/turso` now symlinks to it, existing `~/.config/turso/settings.json` worked unchanged (auth whoami = dezren39). Sandbox gotchas for any reinstall: the official script needs `TAR_OPTIONS=--no-same-owner` (sandbox denies chown, same quirk class as the Nix installer), it performs NO checksum verification, and it always installs 'latest' with no pinning — a reinstall silently upgrades.
