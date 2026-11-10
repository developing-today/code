# Research round 2: Rust toolchain (n15) and MSRV

- Date: 2026-10-11 (fetches about 16:00 UTC)
- Folder: /home/user/work/research-round2/toolchain (all writes are here)
- Repo: read-only. Nothing in /home/user/code, /home/user/work/integrated, /home/user/work/spikes or /home/user/work/spikes-next was changed. `git status` in integrated, run at the start of the session, showed only two pre-existing untracked docs, and pkgs/id had no changes then. I have not written to the repo since.
- Labels: [verified] = seen in command output or source in this session; [inferred] = reasoning from verified facts; [not checked] = not done.

Owner decisions (from the parent, verbatim):
- n15: "n15 move to the very newest toolchain, if that's a patch 1.99.x or newer than 1.99 both are great."
- n14: "n14 in all cases use the git rev unless there's a strong reason not to."

## Questions (verbatim)

1. The newest stable Rust as of today (2026-10-11). Use https://static.rust-lang.org/dist/channel-rust-stable.toml and the versioned manifests. Report the exact version, its date, and whether any newer patch exists.
2. Our repo state, read-only. `pkgs/id/rust-toolchain.toml` (1.97.0). The flake: `flake.nix` and `flake.lock` at the root of `/home/user/work/integrated`. Which rust-overlay revision is locked, and does it contain the newest stable? Which nixpkgs revision is locked, and what rustc does it provide? Read the lock files with `python3 -I` or `jq` only.
3. The exact update steps for a later Phase 0. Name the flake input to update (`nix flake lock --update-input <name>`), and whether `nix` can evaluate the flake offline on this host. Do not run `nix flake update` in the repo. If `nix` is usable, run only `nix eval` on read-only attributes in a copy of the flake directory.
4. Install the newest toolchain into rustup with the minimal profile, so that we can build with it (`rustup toolchain install <version> --profile minimal -c clippy -c rustfmt`). This is a user-level install and is allowed. Then record `rustc -V` and `cargo -V`, and run `cargo --version` against a trivial crate.
5. MSRV and compatibility. Check the minimum supported Rust version of our key dependencies (iroh 1.3, iroh-relay, iroh-blobs 0.103.1, redb 4.3.0, the p2panda git rev 3e84cac, beekem 0.4.0, cbor-core 0.10.1, turso, libsql, ed25519-dalek 2 and 3). Flag any that need a newer toolchain than the newest stable.
6. Note any clippy or rustfmt changes that would affect our lint configuration.

Output: the version, the steps in order, and the risks. Do not modify the repo.

## Short answers

**1. The newest stable is 1.99.0, released 2026-10-01. No newer patch exists.** [verified] RELEASES.md at tag 1.99.0 starts with `Version 1.99.0 (2026-10-01)`, and `channel-rust-stable.toml` has `date = "2026-10-01"`. The rustc build string `1.99.0 (b940084d7 2026-09-28)` carries the commit date, not the release date; task 12's table uses that commit date. HEAD probes on 2026-10-11 for `channel-rust-1.99.1.toml`, `1.99.2`, `1.99.3`, `1.99.4`, `1.99.5`, `1.99.9` and `1.100.0` all returned 404, and the blog release pages and RELEASES.md tags for 1.99.1 and 1.99.2 also returned 404. Two caveats: the stable manifest came through a Varnish cache with age about 14.8 h, so a release from the last few hours could be missed [inferred]; and our pin 1.97.0 is behind 1.97.1 (2026-07-16) [verified].

**2. Both locked rust-overlay revisions contain 1.99.0. The locked nixpkgs revisions provide rustc 1.98.1, not 1.99.0.** [verified] The root flake's id toolchain input is `id-rust-overlay`, locked to oxalica/rust-overlay `4ca2963243427cb86bd6b4d9ca6010db0162a619` (2026-10-09T08:47:34Z). The standalone `pkgs/id/flake.lock` locks `rust-overlay` to `b962e5d0c8cd97196c6a882d659fe451b52ad989` (2026-10-06T08:48:55Z). In both, `rust-bin.stable.latest` evaluates to 1.99.0, and a 1.99.0 toolchain with the repo's four components evaluates to version 1.99.0. The nixpkgs revisions are: root `id-nixpkgs` → nixpkgs-master `cfdb2236cff2a6adaace5a4ef37ea73152d710e9` (2026-10-09), pkgs/id `nixpkgs` → `afe869b7192c6caa674e74227031eaa232a062a7` (2026-10-06), and root `nixpkgs` → nixos-unstable `e7439b6b14ad3cc35d05608ebca9bce01a25f5f8` (2026-10-08). All three give `rustPlatform.rust.rustc` = 1.98.1. The id derivations list rust-overlay's `rust-default-1.97.0` before nixpkgs' `rustc-wrapper-1.98.1` and `cargo-1.98.1`, so the pinned toolchain is probably the compiler that the flake build uses [inferred; not built]. The legacy `pkgs/id/default.nix` does not use rust-overlay, so it gets nixpkgs' `rustPlatform` [verified by reading].

**3. No lock update is needed for the toolchain step.** [verified: manifest component check and offline eval] Change `channel` in `pkgs/id/rust-toolchain.toml` (line 20) from 1.97.0 to 1.99.0. If a newer overlay is wanted, the inputs are `id-rust-overlay` (root flake) and `rust-overlay` (pkgs/id flake), updated with `nix flake lock --update-input <name>`. After any update, compare the locked rev with upstream: nix's `tarball-ttl` is 259200 s (3 days) [verified: `nix show-config`], so a cached tarball can be stale. Offline evaluation works on this host: `nix eval --offline` exited 0 on copies of both flakes and on the two overlay/nixpkgs probes [verified]. The ordered Phase 0 steps are in Recommendation.

**4. Installed 1.99.0 with the minimal profile plus clippy and rustfmt.** [verified] `rustc -V` → `rustc 1.99.0 (b940084d7 2026-09-28)`; `cargo -V` → `cargo 1.99.0 (5f94df478 2026-08-27)`. `cargo --version` on the trivial crate `trivial/hello` gives the same, and its build, run (`Hello, world!`), `clippy -D warnings` and `fmt --check` all exit 0. rustup had no default before, so the install set 1.99.0 as the default. I ran `rustup default none` to restore the previous state [verified: settings.toml before and after].

**5. Nothing needs more than 1.99.0 in what can be checked.** [verified] I queried crates.io `rust_version` for 834 registry (name, version) pairs from both locks. 603 declare one, the highest is sqlx 0.9.0 at 1.94.0, and no pair is yanked or failed to look up. The p2panda crates at rev 3e84cac all declare `rust-version = "1.96"` (fetched from GitHub). Per crate: iroh, iroh-relay, iroh-blobs 0.103.1, iroh-tickets, iroh-docs and iroh-gossip → 1.91; redb 4.3.0 → 1.90 (redb 3.1.3 → 1.89, also in the pkgs/id lock); cbor-core 0.10.1 → 1.88; ed25519-dalek 2.2.0 → 1.81 and 3.0.0 → 1.85; tokio 1.53.2 → 1.71; beekem 0.4.0 → 1.90.0 (not in either lock). No declared MSRV: dcbor 0.25.2, libsql 0.9.30, distributed-topic-tracker 0.3.5, and turso 0.8.2 (not in either lock). These need a compile to check. Our declared MSRV is 1.91.0 (`pkgs/id/Cargo.toml:5`), below p2panda's 1.96.

**6. rustfmt: no change for our code. Clippy: 1.99 adds lints that our config enables, and about 50 call sites would trigger one.** [verified; firing on the repo's sites inferred] rustfmt 1.97.0 and 1.99.0 both give zero diffs on 66 files with `--edition 2024`. Our `[lints.clippy]` sets `all`, `pedantic`, `nursery` and `cargo` to warn (`Cargo.toml:116–119`). A probe on a trivial crate shows four warnings that appear only on 1.99.0: `assert_is_empty`, `chunks_exact_to_as_chunks`, `unused_async_trait_impl` and `with_capacity_zero`. Grep finds 50 `assert!(...is_empty())` sites in pkgs/id and 2 `chunks_exact(2)` sites; whether the lints fire on them is not checked. The repo's `just clippy-lint` has no `-D warnings`, so these would not fail it, but task 12's "clippy reports nothing new" would. The 1.98 change that turns some `ambiguous_glob_imports` cases into hard errors could affect our 71 glob imports; that is not checked.

## Evidence

Commands are abbreviated. Full logs are in `logs/`, parsed data in `data/`, and downloads in `dl/`, each download in its own directory.

### E1. Newest stable (Q1)
- `curl -sS -H 'Cache-Control: no-cache' -D dl/stable.headers -o dl/stable/channel-rust-stable.toml https://static.rust-lang.org/dist/channel-rust-stable.toml` → HTTP 200, 943,486 bytes, sha256 `ce6dddc886364f8d786514771212cebe9b731ba82d6b859951c6b0ccc516b6a2`, `last-modified: Thu, 01 Oct 2026 12:38:04 GMT`, `age: 53507`, `x-cache: HIT` (fetched 2026-10-11 16:06 UTC).
- `python3 -I scripts/manifest_summary.py dl/stable/channel-rust-stable.toml` → `manifest-version: 2`, `date: 2026-10-01`, `rustc: 1.99.0 (b940084d7 2026-09-28)`, `cargo: 0.100.0 (5f94df478 2026-08-27)`, `clippy-preview: 0.1.99`, `rustfmt-preview: 1.10.0`, `rust-std` and `rust-src: 1.99.0 (b940084d7 2026-09-28)`.
- Component availability on x86_64, from `python3 -I` on the same manifest: `rustfmt-preview` 1.10.0 True; `rust-analyzer-preview` 0.0.0 True; `clippy-preview` 0.1.99 True; `llvm-tools-preview` 1.99.0 True; `rustc` True; `cargo` True. (My first pass used `rustfmt` and `rust-analyzer` as keys and returned None; the manifest uses the `-preview` names.)
- HEAD probes (`curl -s -o /dev/null -w '%{http_code}' -I https://static.rust-lang.org/dist/channel-rust-<v>.toml`): 1.97.0 200, 1.98.0 200, 1.98.1 200, 1.98.2 404, 1.99.0 200, 1.99.1 404, 1.99.2 404, 1.99.3 404, 1.99.4 404, 1.99.5 404, 1.99.9 404, 1.100.0 404.
- `https://blog.rust-lang.org/releases/1.99.1/` 404; `…/1.99.2/` 404; `https://raw.githubusercontent.com/rust-lang/rust/1.99.1/RELEASES.md` 404; `…/1.99.2/RELEASES.md` 404.
- `dl/rust-1.99.0-releases/RELEASES.md` (tag 1.99.0, 930,191 bytes): `Version 1.99.0 (2026-10-01)` (line 1), `Version 1.98.1 (2026-09-03)` (line 131), `Version 1.98.0 (2026-08-20)` (line 138), `Version 1.97.1 (2026-07-16)` (line 237), `Version 1.97.0 (2026-07-09)` (line 247).

### E2. Repo state (Q2)
- Pins: `pkgs/id/rust-toolchain.toml:20` `channel = "1.97.0"`; line 22 `components = ["rustfmt", "clippy", "rust-analyzer", "llvm-tools"]`; line 23 `profile = "default"`. The same file exists under /home/user/code and /home/user/work/integrated [verified: read].
- Legacy files: a `rust-toolchain` file (no `.toml`) containing `nightly-2023-07-20` sits in /home/user/code and /home/user/work/integrated. Running rustup from /home/user/code printed `warn: both /home/user/code/rust-toolchain and /home/user/code/pkgs/id/rust-toolchain.toml exist; using contents of /home/user/code/rust-toolchain` [verified].
- `jq -c '.root as $r | .nodes[$r].inputs' /home/user/work/integrated/flake.lock` → `id-rust-overlay → id-rust-overlay`, `id-nixpkgs → ["nixpkgs-master"]`, `nixpkgs → ["nixpkgs-unstable"]`, and `id-flake-parts`/`id-flake-utils`/`id-systems` follow other inputs. Locked nodes (jq):
  - `id-rust-overlay`: github oxalica/rust-overlay, rev `4ca2963243427cb86bd6b4d9ca6010db0162a619`, lastModified 2026-10-09T08:47:34Z.
  - `nixpkgs-master`: github NixOS/nixpkgs, rev `cfdb2236cff2a6adaace5a4ef37ea73152d710e9`, 2026-10-09T22:43:50Z.
  - `nixpkgs-unstable`: nixos-unstable `e7439b6b14ad3cc35d05608ebca9bce01a25f5f8`, 2026-10-08T10:40:40Z.
  - Other overlay users, not id: `rust-flake → rust-overlay` `6b5c52313aaf3f3e1a0a6757bb89846edfb5195c` (2025-12-27); `yazi → rust-overlay_2` `7470661ba2b51156048af08464b82020223dc942` (2026-10-03).
- `jq` on `/home/user/work/integrated/pkgs/id/flake.lock`: `rust-overlay` `b962e5d0c8cd97196c6a882d659fe451b52ad989` (2026-10-06T08:48:55Z); `nixpkgs` `afe869b7192c6caa674e74227031eaa232a062a7` (2026-10-06T12:14:35Z).
- Wiring: root `flake.nix:313–314` declares `id-rust-overlay` (url `github:oxalica/rust-overlay`). `lib/default.nix:593` `make-id` imports `../pkgs/id/flake.nix` and maps its `rust-overlay` and `nixpkgs` inputs to `id-rust-overlay` and `id-nixpkgs`. `pkgs/id/nix-common.nix:20` `rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;`. `pkgs/id/flake.nix:445, 576, 634` put `rustToolchain` in `nativeBuildInputs`.
- Offline eval (`scripts/rust_toolchain_probe.nix`, `OV_REV=… NP_REV=… nix eval --offline --impure --raw --file …`), exit 0 for each pair:
  - root pair (4ca2963, cfdb2236): `rust-bin.stable.latest=1.99.0`, `rust-bin.stable.1.99.0=present`, `rust-bin.stable.1.97.0=present`, `rust-bin 1.99.0 default(+repo components).version=1.99.0`, `nixpkgs.rustPlatform.rust.rustc.version=1.98.1`, `nixpkgs.rustPlatform.rust.cargo.version=1.98.1`.
  - pkgs/id pair (b962e5d, afe869b): the same results.
  - root nixpkgs e7439b6b with 4ca2963: `rustPlatform.rust.rustc.version=1.98.1`.
- Derivation inputs by name (`scripts/pkgid_native_inputs.nix` on the pkgs/id copy; `scripts/root_native_inputs.nix` on the root copy; both `nix eval --offline`, exit 0, no build and no instantiation). pkgs/id copy: `packages=default,id-lib,id-web`; `id-lib.nativeBuildInputs=[pkg-config-wrapper-0.29.2, cmake-4.4.3, rust-default-1.97.0, auditable-cargo-1.98.1, cargo-build-hook.sh, cargo-check-hook.sh, cargo-install-hook.sh, cargo-setup-hook.sh, rustc-wrapper-1.98.1, cargo-1.98.1]`. Root copy: `id-lib` and `id-web` (which adds bun) carry the same nixpkgs-side entries with `rust-default-1.97.0`.
- nixpkgs source (cfdb2236, `/nix/store/zqrnilbnkjj5d6kaj6r1ilzyjmd8vfwy-source`): `pkgs/build-support/rust/build-rust-package/default.nix` line 69 `auditable ? !cargo-auditable.meta.broken`; lines 148–160 `nativeBuildInputs = nativeBuildInputs ++ [ … cargoSetupHook rustc cargo ]`. Caller inputs come first, so PATH should prefer the pinned toolchain [inferred; not verified by a build].
- Legacy path: `pkgs/id/default.nix:12` `pkgs ? import <nixpkgs> { }`; `:25` `default = pkgs.rustPlatform.buildRustPackage {` with `nativeBuildInputs = with pkgs; [ pkg-config ]`, no overlay and no rustToolchain. `grep -nE 'overlay|rust-bin|rustToolchain|rustc|nixpkgs' default.nix shell.nix` shows no rust-overlay in default.nix; shell.nix uses rust-overlay at lines 34–38 [verified].
- Declared MSRV: `pkgs/id/Cargo.toml:5` `rust-version = "1.91.0"`; `pkgs/id/clippy.toml:14` `msrv = "1.91.0"`. Lints: `Cargo.toml` `[lints.rust]` at line 99 (`unsafe_code = "warn"` at 101); `[lints.clippy]` at line 114; `all`, `pedantic`, `nursery` and `cargo` at warn on lines 116–119; deny on lines 123–128 (`unwrap_used`, `expect_used`, `panic`, `unimplemented`, `todo`, `dbg_macro`) [verified: grep and sed].
- Gate: `pkgs/id/justfile:163–164` `clippy-lint:` runs `cargo clippy --all-targets --all-features` with no `-D warnings`. `pkgs/id/check-all:91` runs with `-D warnings || true` [verified].
- Lock contents (`scripts/lock_packages.py` via `python3 -I` on each Cargo.lock): `pkgs/id/Cargo.lock` (lockfile v4) has 752 packages, 751 registry, 0 git, so p2panda is not in it. `spikes/Cargo.lock` has 584 packages, 569 registry, 7 git (p2panda at 3e84cac) [verified].
- Git: integrated HEAD `d702af03`. `git --no-optional-locks status --porcelain` lists only two untracked docs, and pkgs/id has no changes [verified].
- nix: `nix --version` = 2.35pre20260619; `nix show-config` shows flakes and nix-command enabled and `tarball-ttl = 259200` [verified].

### E3. Offline evaluation and copies (Q3)
- `git -C /home/user/work/integrated archive HEAD pkgs/id | tar -x -C copy`; `cmp` shows the copy's flake.lock matches the working tree. `git archive HEAD | tar -x -C copy-root` gave 103 MB; `git ls-files | grep -cE '\.(iso|qcow2)$'` = 0, so the large images are untracked and were not copied [verified].
- `ROOT_FLAKE=path:…/copy-root nix eval --offline --impure --raw --file scripts/root_native_inputs.nix` → exit 0, with warnings only (for example `input 'git-hooks' has an override for a non-existent input 'gitignore'`).
- `PKGID_FLAKE=path:…/copy/pkgs/id nix eval --offline --impure --raw --file scripts/pkgid_native_inputs.nix` → exit 0.
- Not run: `nix flake lock --update-input …`, `nix flake update`, `nix build`, `nix develop`, and anything in the repo.

### E4. Install and trivial crate (Q4)
- `nohup rustup toolchain install 1.99.0 --profile minimal -c clippy -c rustfmt` → log: `latest update on 2026-10-01 for version 1.99.0 (b940084d7 2026-09-28)`; `1.99.0-x86_64-unknown-linux-gnu installed - rustc 1.99.0 (b940084d7 2026-09-28)`; `default toolchain set to 1.99.0-x86_64-unknown-linux-gnu` (logs/rustup-install-1.99.0.log) [verified].
- Before the install, `rustup toolchain list` showed `nightly-2023-07-20-x86_64-unknown-linux-gnu` and `1.97.0-x86_64-unknown-linux-gnu`, neither marked default. After the install it showed `1.99.0 (active, default)`.
- `rustup default none` → `info: default toolchain unset`. `~/.rustup/settings.toml`: before, `default_toolchain = "1.99.0-x86_64-unknown-linux-gnu"`; after, no default line [verified].
- With PATH set to the toolchain bin (`dirname "$(rustup which --toolchain 1.99.0 cargo)"`): `rustc -V` = `rustc 1.99.0 (b940084d7 2026-09-28)`; `cargo -V` = `cargo 1.99.0 (5f94df478 2026-08-27)`; `cargo clippy -V` = `clippy 0.1.99 (b940084d7e 2026-09-28)`; `rustfmt --version` = `rustfmt 1.10.0-stable (b940084d7e 2026-09-28)` [verified].
- `trivial/hello` (`cargo new hello --vcs none`): `cargo --version` = `cargo 1.99.0 (5f94df478 2026-08-27)`; `cargo build` exit 0; `./target/debug/hello` prints `Hello, world!`; `cargo clippy -- -D warnings` exit 0; `cargo fmt --check` exit 0 [verified].

### E5. MSRV (Q5)
- Method: `scripts/lock_packages.py` (`python3 -I`, tomllib) read both Cargo.lock files into `data/pkgid-lock.json` and `data/spikes-lock.json`. `scripts/msrv_crates_io.py` queried `https://crates.io/api/v1/crates/<name>/<version>` once per second with a descriptive User-Agent, for the 834 registry pairs (2026-10-11, 0 errors). `scripts/msrv_report.py` summarised the results into `data/msrv-report.txt`.
- Scan summary: 834 pairs, 603 with rust_version, 231 unspecified, 0 errors, 0 yanked. Highest in the union: sqlx 0.9.0 → 1.94.0. Per lock: pkgs/id has 751 registry crates, max `wasmtime-internal-versioned-export-macros 43.0.2` → 1.91.0, 202 unspecified; spikes has 569 registry crates, max `sqlx-sqlite 0.9.0` → 1.94.0, 156 unspecified. Crates needing more than 1.99.0: none.
- Key crates (crates.io `version.rust_version` and `yanked`):

| Crate (version) | In lock | rust_version | Yanked | vs 1.99.0 |
|---|---|---|---|---|
| iroh 1.3.0 | both | 1.91 | no | ok |
| iroh-relay 1.3.0 | both | 1.91 | no | ok |
| iroh-blobs 0.103.1 | both | 1.91 | no | ok |
| iroh-tickets 1.0.0 | both | 1.91 | no | ok |
| iroh-docs 0.101.0 | pkgs/id | 1.91 | no | ok |
| iroh-gossip 0.101.0 | pkgs/id | 1.91 | no | ok |
| redb 4.3.0 | spikes | 1.90 | no | ok |
| redb 3.1.3 | pkgs/id | 1.89 | no | ok (a second redb copy in pkgs/id) |
| cbor-core 0.10.1 | spikes | 1.88 | no | ok |
| dcbor 0.25.2 | spikes | unspecified | no | unknown |
| ed25519-dalek 2.2.0 | spikes | 1.81 | no | ok |
| ed25519-dalek 3.0.0 | both | 1.85 | no | ok |
| libsql 0.9.30 | pkgs/id | unspecified | no | unknown (C code; task 12 risk) |
| tokio 1.53.2 | both | 1.71 | no | ok |
| distributed-topic-tracker 0.3.5 | pkgs/id | unspecified | no | unknown |
| p2panda-{core, auth, encryption, sync, store, stream} 0.7.1 @ 3e84cac | spikes (git) | 1.96 | n/a | ok, but needs 1.96 |
| beekem 0.4.0 | neither | 1.90.0 | no | ok (not a current dependency) |
| turso 0.8.2 | neither | unspecified | no | unknown (not a current dependency) |

- p2panda: `dl/p2panda-p2panda-<crate>/Cargo.toml` line 15 `rust-version = "1.96"` for each of the six crates. The root `dl/p2panda-root-Cargo.toml` declares no rust-version [verified: GitHub raw at 3e84cac, 2026-10-11].
- spikes: `spikes/Cargo.toml:7` `rust-version = "1.96"` [verified].
- Product: `pkgs/id/Cargo.toml:5` (1.91.0) and `pkgs/id/clippy.toml:14` (msrv 1.91.0) are both below 1.96 [verified].

### E6. rustfmt and clippy (Q6)
- rustfmt: `rustfmt --edition 2024 --check` over the 66 `.rs` files in `copy/pkgs/id` (the same invocation as the `rustfmt-check` in `pkgs/id/flake.nix:448`). 1.97.0: exit 0, 0 "Diff in" blocks. 1.99.0: exit 0, 0 blocks (`logs/rustfmt-*.txt`) [verified].
- Changelogs: `dl/clippy-changelog/CHANGELOG.md` (rust-clippy master, fetched 2026-10-11), sections "Rust 1.99" (line 11) and "Rust 1.98" (line 99), extracted to `data/clippy-1.99.md` and `data/clippy-1.98.md`.
  - 1.99 New Lints: assert_is_empty (pedantic), nonnull_unchecked_on_box_ptr (complexity), block_scrutinee (suspicious), definition_in_module_root, rest_pattern_accessible_field and unnecessary_rest_pattern (restriction), manual_bit_width (pedantic), mismatched_bit_width_type (suspicious).
  - 1.98 New Lints: unnecessary_unwrap_unchecked (complexity), chunks_exact_to_as_chunks (style), by_ref_peekable_peek (suspicious), with_capacity_zero (pedantic), manual_isolate_lowest_one (complexity), for_unbounded_range (suspicious), unused_async_trait_impl (pedantic).
  - 1.98 Moves and Deprecations: `empty_enums` moved pedantic → nursery (both are enabled in our config, so no effect); `from_iter_instead_of_collect` deprecated (not in our config). 1.99 has no Moves section. None of the lint names in our `[lints]` tables was renamed or removed [verified].
- Clippy probe (`trivial/lintprobe/src/lib.rs`, eight small functions covering the candidate patterns; `cargo clippy -- -W clippy::all -W clippy::pedantic -W clippy::nursery`; separate target dirs and `RUSTC` pinned per toolchain):
  - 1.97.0 warnings: must_use (5), const fn (2), `&Vec` (1), missing `# Panics` (1), usize→u32 cast (1).
  - 1.99.0 adds: "used `assert!` to check that a value is empty"; "using `chunks_exact` with a constant chunk size"; "unused `async` for async trait impl function with no `.await` statements"; "calling `with_capacity(0)` is equivalent to `new()`" [verified: `logs/lintprobe-1.97.0.txt`, `logs/lintprobe-1.99.0.txt`].
- Repo greps on `copy/pkgs/id` (`src`, `tests`, `examples`) [verified, grep]:
  - `assert!(.*\.is_empty\(\))`: 50 (48 in src).
  - `chunks_exact` or `as_chunks`: 2. `src/world_session.rs:2065` is `.chunks_exact(2)` inside a `.map`; `src/world.rs:3107` is `bytes.chunks_exact(2)` inside a `zip`.
  - `with_capacity(0)`: 0. `unwrap_unchecked`: 0. `chunks(0)` or `windows(0)`: 0. Footnote syntax in doc comments: 0. `#[path`: 0. `no_mangle` or `export_name`: 0. `cfg_select!`: 0.
  - `use …::*;`: 71.
  - `c_void`: 4, all `as *const libc::c_void` pointer casts (`src/world_native.rs:390, 421, 444, 500`), not return types.
- RELEASES.md 1.98 notes (`dl/rust-1.98.0-releases/RELEASES.md`, extracted to `data/relnotes-1.98.0.md`): line 11 adds the warn-by-default `c_void_returns` lint for `core::ffi::c_void` as a return type; line 9 adds the deny-by-default `invalid_runtime_symbol_definitions`; line 79 converts some `ambiguous_glob_imports` cases into hard errors; line 91 emits `unsafe_code` for all unsafe attributes; line 94 has rustfmt discover modules inside `cfg_select!` [verified].
- RELEASES.md 1.99 notes (`data/relnotes-1.99.0.md`): lines 8–24 cover rustc lints, including `unconditional_panic` extended to zero-size chunks and windows (deny-by-default), POSIX runtime symbols, `#[path]` on inline modules, `unreachable_cfg_select_predicates` in the unused group, and an invalid doc attribute on a macro invocation. Line 96 adds the rustdoc `unused_footnote_definition` lint. Lines 116–118 add `semicolon_in_expressions_from_non_local_macros`, a warning lint for macros from other crates. Line 88 disables incremental compilation by default in CI; our `.cargo/config.toml` sets `[build] incremental = true` explicitly [verified].

## Options

| Option | Toolchain | Lock change | Gain | Cost or risk |
|---|---|---|---|---|
| A (recommended, n15) | 1.99.0 in `pkgs/id/rust-toolchain.toml` | none; both overlay revisions already have 1.99.0 | Newest stable; owner's decision; eval passes offline; rustfmt clean | About 50 `assert_is_empty` (pedantic) warnings and about 2 `chunks_exact` warnings; possible 1.98 glob-import errors; no compile run yet |
| B | 1.98.1 | none | One step back from newest | Not what the owner asked for; the same clippy work applies |
| C | stay at 1.97.0 (1.97.1 exists) | none | No churn | Contradicts n15; misses the 1.98 and 1.99 fixes; the MSRV raise to 1.96 is still needed for p2panda |
| D | A plus a nixpkgs bump: `nix flake lock --update-input id-nixpkgs` (root) or `nixpkgs` (pkgs/id) | yes | The legacy `default.nix` and the nixpkgs-side rustc move | Not needed for the flake build; whether a newer nixpkgs has rustc 1.99.0 is not checked; more lock churn |

## Recommendation

Take option A. The toolchain step needs no lock update, the newest stable is 1.99.0, and no crate with a declared MSRV needs more than 1.99.0. The p2panda git rev (n14) needs 1.96, which 1.99.0 satisfies, so the git-rev decision stands. Phase 0 steps, in order (for later; none of these were run):

1. Edit `pkgs/id/rust-toolchain.toml` line 20 to `channel = "1.99.0"`. Keep the components and profile.
2. Do not run `nix flake update`. Only if a newer overlay is wanted: `nix flake lock --update-input id-rust-overlay` (root) or `nix flake lock --update-input rust-overlay` (pkgs/id). Then check the locked rev against `git ls-remote https://github.com/oxalica/rust-overlay refs/heads/master`, because of the 3-day tarball TTL.
3. Edit `pkgs/id/Cargo.toml:5` to `rust-version = "1.96"` and `pkgs/id/clippy.toml:14` to `msrv = "1.96"` (task 01). This keeps clippy's MSRV-gated lints in step with the declared MSRV.
4. Offline check in a copy, not the repo: `git -C /home/user/work/integrated archive HEAD pkgs/id | tar -x -C <copy>`, apply steps 1 and 3 to the copy, then `PKGID_FLAKE=path:<copy>/pkgs/id nix eval --offline --impure --raw --file scripts/pkgid_native_inputs.nix` and expect `rust-default-1.99.0` in the list. Use `--no-write-lock-file` for any further eval.
5. Compile and lint gates with the user toolchain, using a fresh target dir per toolchain (E0514 risk). `TC=$(dirname "$(rustup which --toolchain 1.99.0 cargo)")`, then in `pkgs/id`: `PATH="$TC:$PATH" RUSTC="$TC/rustc" CARGO_TARGET_DIR=<fresh> cargo clippy --all-targets --all-features -- -D warnings`; `cargo test --lib -- --skip world_compile --skip test_worlds_compile_roc_sources_on_the_fly --skip test_world_module_shared_over_iroh_and_web_presentations`; `cargo test --doc`; and `rustfmt --edition 2024 --check`. Expect the lint set in E6. This is a heavy build, so coordinate with the other agents.
6. Decide the roughly 50 `assert_is_empty` sites: fix them, or allow the lint in `[lints.clippy]` with a stated reason. Decide the two `chunks_exact` sites the same way.
7. Nix build (owner or CI only; agents must not run it): `nix build` for `id-lib` and `id-web`, and confirm from the build log that rustc is `rust-default-1.99.0`, not nixpkgs' 1.98.1.
8. The legacy `pkgs/id/default.nix` ignores the pin. Either route it through rust-overlay, or document that `nix-build` is not the supported path.
9. Commit the toolchain change on its own, with the version table in the body (task 12 acceptance).

What would change the recommendation: a compile error from the 1.98 glob-import change or from the libsql C build; a crate with no declared MSRV that fails on 1.99.0; a Nix build log showing that the 1.98.1 nixpkgs rustc is the compiler; or a 1.99.x patch appearing before Phase 0 (that changes the version string, not the choice).

## Not verified

- No `cargo build`, clippy or test run of pkgs/id on 1.99.0. The counts in E6 come from grep and a probe crate, not from a clippy run on the product.
- No `nix build`, `nix develop` or lock update. The compiler the flake build actually uses, and whether 1.99.0 binaries are cached, are inferred from eval only.
- MSRV for 202 pkgs/id and 156 spikes registry crates with no declared rust_version, including dcbor, libsql, distributed-topic-tracker and turso. Only a compile on 1.99.0 can check them.
- Whether the 1.99 `semicolon_in_expressions_from_non_local_macros` warning or the 1.98 glob-import hard error hits our code. Only the grep counts are known.
- Freshness: the stable manifest came through a cache with age about 14.8 h, so the patch probes could miss a release from the last few hours. Re-check before Phase 0.
- Whether a newer nixpkgs revision provides rustc 1.99.0. The task 12 doc says the local store has one, but no locked revision does.
- The NIX_PATH nixpkgs on this host, which `default.nix` uses, is unknown.
- rust-analyzer and llvm-tools availability is confirmed from the manifest, not from an install. Only clippy and rustfmt were installed.
- The spikes lock has a source-less `p2panda-encryption 0.7.1` entry next to the git one. Not examined.
- Clippy's use of `clippy.toml` msrv is inferred from clippy's config semantics. Not run.

## Files

- `RESULTS.md` (this file).
- `scripts/`: python3 -I and nix eval scripts (`manifest_summary.py`, `component_check.py`, `lock_packages.py`, `msrv_crates_io.py`, `msrv_report.py`, `rust_overlay_probe.nix`, `rust_toolchain_probe.nix`, `pkgid_native_inputs.nix`, `root_native_inputs.nix`).
- `data/`: parsed locks, crates.io results, and release-note extracts.
- `logs/`: rustup install, nix evals, clippy and rustfmt runs.
- `dl/`: downloaded manifests and release files, each in its own directory.
- `copy/` and `copy-root/`: git-archive copies of HEAD. Nothing was written back to the repo.
- `trivial/`: the `hello` and `lintprobe` crates. Their `target/` dirs are inside this folder.
