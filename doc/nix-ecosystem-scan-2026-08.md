# Nix ecosystem scan — August 2026

> Aggregated research on nixpkgs, NixOS, Nix CLI/store, Lix, binary caches, Clan,
> iroh and roc. Compiled 2026-08-24 from release notes, blogs, discourse threads and
> project docs. Relevant to this repo: large multi-host flake, ~50 inputs, just moved
> from a Nov-2025 fork snapshot to `nixos-unstable` + patchfile.

---

## 1. Release cadence

| Branch | Released | Codename | Status |
|---|---|---|---|
| 25.05 | 2025-05-23 | Warbler | EOL 2025-12-31 |
| **25.11** | 2025-11-30 | Xantusia | **EOL 2026-06-30 — expired** |
| **26.05** | 2026-05-30 | Yarara | **current stable** until 2026-12-31 |
| unstable → 26.11 | branch-off ~late Oct 2026 | Zokor | current unstable |

Headlines: 25.11 = GNOME 49 (no X11), LLVM 21/GCC 14/CMake 4, kernel 6.17, PostgreSQL
17, `nixfmt` stable as `pkgs.nixfmt`, `nodePackages` deprecation announced.
26.05 = **systemd initrd default** (scripted deprecated, removal in 26.11),
**dbus-broker default** (switch inhibitor → reboot required), GCC 15/glibc 2.42,
kernel 6.18, Node 24 LTS, `system.nix` channel-less entry point, nspawn-based VM test
driver, `nodePackages`/yarn2nix/node2nix removed entirely, **last release with
x86_64-darwin support**. 26.11 also discontinues `.tar.xz` channel tarballs → migrate
to `nixexprs.tar.zst`.

## 2. Nixpkgs structure

- **pkgs/by-name (RFC 140)**: mandatory for new packages since mid-2024; bulk
  migration done; what remains in `all-packages.nix` is mostly override-wrappers.
  Open issue [#537188](https://github.com/NixOS/nixpkgs/issues/537188) (2026-06) +
  [`overrideVariant` PR #537454](https://github.com/NixOS/nixpkgs/pull/537454)
  address loss of `.override` during migrations — **renames can hard-error your
  `.override` calls**.
- **Modular services framework**: service definitions live *with packages* as
  `<pkg>.passthru.services.default` (`_class = "service"`), configured via
  `system.services.<name>`. Portable base moved to `lib/services/lib.nix`
  ([PR #506519](https://github.com/NixOS/nixpkgs/pull/506519), Apr 2026) + exposed as
  `lib.services`. **Home-manager adopted it in HM 26.05**: new `home.services.*`
  namespace lifts package services into `systemd.user.*` — this is why HM master's
  `modules/services-modular/default.nix` imports `pkgs.path + "/lib/services/lib.nix"`
  (the failure we hit when pairing latest HM with our old fork snapshot).
- **Alias lifecycle**: alias → `warnAlias` → `throw` → removal
  (`remove-old-aliases.py`); policy friction documented in
  [#493853](https://github.com/NixOS/nixpkgs/issues/493853) (warn immediately vs
  after backport window). [RFC 180](https://github.com/NixOS/rfcs/pull/180) governs
  broken/unmaintained removals (broken ≥1 year ⇒ throw; explains the Feb–May 2026
  removal wave that hit hexchat, geda, nitrogen etc.). Deprecation-guidelines PR
  [#520061](https://github.com/NixOS/nixpkgs/pull/520061) requires actionable messages.
- **Package-set flattening**: `xorg.*` set deprecated (packages at top level with
  lowercase names — `libx11`, `xwininfo`…); MATE/Xfce scopes flattened; `qt5.full` /
  `qt6.full` aliases removed; Qt5 Plasma/Gear removed (→ `kdePackages`).
  `qt6Packages` decoupled from `kdePackages`.
- RFC process itself moved to on-demand workflow (May 2025).

## 3. NixOS module changes relevant to this config

Already handled today: `networking.wireless.userControlled` rename (+ group fixed to
`wpa_supplicant`), `programs.solaar` upstreamed (flake dropped), xorg renames.

Also worth knowing:
- `nixos-rebuild-ng` default since 25.11; bash version + opt-out option **removed in
  26.05**. Needs `git` installed for flake use.
- systemd stage-1 initrd default in 26.05 — check LUKS device naming before first
  boot of a 26.05+ generation; `/dev/root` gone.
- Switch **inhibitors**: refuse switch when systemd version changed (override
  `NIXOS_NO_CHECK=1`). Activation-script unit reload/restart deprecated (26.11 removal).
- Module renames: `stalwart-mail`→`stalwart`, `jellyseerr`→`seerr`,
  `eintopf`→`lauti`; `services.promtail`/`statsd`/`uptime` removed;
  `profiles/hardened`, `linux_hardened`, `linux-rt` kernels gone; NVIDIA pre-Maxwell
  needs `nvidiaPackages.legacy_580`.
- New modules: `programs.atuin`, `services.logiops`, `services.dunst`,
  `services.meshtasticd`, `boot.loader.refind`, `services.firewalld`.

## 4. Nix CLI / store

Latest stable line ~2.34.x (Feb 2026); your system runs 2.35pre.

Per-release highlights:
- **2.30** (Jul 2025): stack-sampling eval profiler (`--eval-profiler flamegraph`),
  `json-log-path`, IFD tracing warnings, `nix flake archive --no-check-sigs`.
- **2.31** (Aug 2025): **WAL mode for SQLite caches**, **parallel GC marking**
  (bdwgc), `nix flake prefetch-inputs`, git-hashing store objects extended to
  SHA-256, `user@host:port` store URIs, `warn-short-path-literals`.
- **2.32** (Oct 2025): daemon protocol <2.0 clients dropped; lazy C API accessors;
  HTTP binary-cache metadata compression (`narinfo-compression`, `ls-compression`,
  `log-compression`); temp build dirs no longer embed derivation names (fixes long-name
  failures); **`external-builders` setting** (QEMU-style foreign builders);
  attrset-merge memory optimization; `nix flake check` skips substitutable derivations;
  AST bump allocator begun.
- **2.34** (Feb 2026): **`nix store roots-daemon`** — serves GC roots over a socket so
  GC works without giving the daemon `CAP_SYS_PTRACE`; **`nix-nswrapper`** — run the
  whole daemon unprivileged in a user namespace (`/etc/subuid`/`subgid` + new
  `nix.daemonUser`/`nix.daemonGroup` NixOS settings); trusted-users FileTransfer fix.
- CA derivations still experimental; git-hashing derivations progressing.

## 5. Alternative implementations & caches

| Project | State Aug 2026 |
|---|---|
| **Lix** | 2.95 "Kakigōri" (Mar 2026), 6th major release; fork point CppNix 2.18; focus = code cleanup/stability, Cap'n Proto RPC replacing bespoke protocol, daemon cache dir moved to `/var/cache/nix`; deliberately **no lazy trees** (own replacement planned), **CA derivations rewrite planned** (none usable meanwhile), no libgit2; REPL improvements; 8–20 % faster than 2.18 |
| **Determinate Nix** | Determinate Systems' validated downstream of CNix (commercial support angle) |
| **snix/Tvix lineage** | Rust ground-up reimplementation, modular; nar-bridge included in cache-shootout benchmarks |
| **Caches** | Lix docs now steer people away from perl `nix-serve` → attic / harmonia / S3+garage. [Mic92/cache-shootout](https://github.com/Mic92/cache-shootout) (Apr 2026) benchmarked harmonia, nix-serve(-ng), ncps (proxy cache), attic (sqlite/S3, zstd chunks), snix nar-bridge, plain nginx/S3 — performance bands split on "serving bare files vs reassembling NARs". Note: **Determinate Systems maintains an attic fork** (last push Feb 2026); zhaofengli's original is slow-moving |

## 6. Clan (clan.lol)

- Version scheme now tracks NixOS releases: **26.05 stable** (~mid-2026) after 25.11
  (Dec 2025); 41 contributors, 2,849 commits/1,086 PRs in that cycle; primary forge is
  Gitea at `git.clan.lol/clan/clan-core` (GitHub mirror read-only); commits landed the
  day this scan was written.
- Model: inventory instances + services with roles/tags and typed exports; ADRs in-tree.
- Secrets: `vars` generators, sops backend default, experimental plain-age backend.
- Deployment: `clan machines install|update|build`; flagship 26.05 fix registers the
  generation **in the bootloader before live activation** (survives failed activation).
- Networking: layered fallback transports tried in priority order —
  **`p2p-ssh-iroh`** (new/experimental, SSH-over-QUIC via iroh) → internet → wireguard
  → zerotier (breaking multi-instance rework) → mycelium → yggdrasil → tor onion.
  Plus data-mesher/dm-dns P2P service discovery.
- Relevance here: this flake already consumes clan-core; the iroh transport pairs with
  the iroh findings below.

## 7. iroh (n0-computer)

- **1.0 landed June 15 2026** ("dial keys, not IPs") after 65 prereleases; patching
  through 1.0.3 by Jul 20. Wire-compat guaranteed across v1 minors/languages.
- Identity renamed to **EndpointId**; addressing via DNS/pkarr `(endpoint_id,
  relay_url)`; own QUIC multipath + NAT-traversal implementations; WASM/browser
  support; LAN discovery without internet.
- `iroh-blobs` rewritten at v0.90 (BLAKE3 content addressing), separate crate at
  v0.103, not yet 1.0; Willow experiments live separately.
- Relay model: public relays serve v0.9x clients only until **Sept 30 2026** — pin or
  upgrade accordingly if you self-host relays.
- Nix tie-in: [`ysndr/n2p`](https://github.com/ysndr/n2p) bridges `nix copy` between
  machines' daemons over an iroh tunnel (no SSH server needed) — a natural companion
  to a self-hosted attic/harmonia for syncing stores off-LAN.

## 8. roc-lang

- The Rust compiler is frozen at **alpha4-rolling (Aug 2025)**. The scratch rewrite
  (in Zig — not self-hosting) reached near feature parity around Mar 2026; rtfeldman
  targets Roc **0.1 before end of 2026**. Nightlies are daily from green main
  (`roc-lang/nightlies`).
- Jun 5 2026 commit deleted old-compiler artifacts including root `flake.nix` — which
  is exactly why our input broke. Official distribution for Nix users is now
  [`roc-lang/roc-overlay`](https://github.com/roc-lang/roc-overlay) (created Jul 29
  2026): mirrors prebuilt nightlies with SHA validation, daily automated updates,
  patchelf-wrapped Linux binaries specifically for NixOS, tags per nightly for pinning
  (`?ref=nightly-2026-08-07-8d23662` style). We already consume it via
  `inputs.roc.packages.<system>.nightly`.
- Ecosystem: URL/tarball content-addressed platform packages, `roc-start` scaffolder,
  community build tool `rbt`, basic-webserver migrated to the new compiler (Jun 2026).
  Expect churn until 0.1 lands.

## 9. Action items surfaced by this scan

1. ~~Repin off EOL 25.11~~ — done (unstable + patchfile; consider also testing a
   `nixos-26.05` stable pin variant).
2. Watch for `xorg.*` throws (currently warnings) — already migrated.
3. dbus-broker switch on next stable jump → plan a reboot (switch inhibitor).
4. Consider `home.services.*` modular services instead of hand-written user units.
5. If builds ever need QEMU cross targets, look at 2.32 `external-builders`.
6. Self-hosted cache: attic (or its DS fork) + S3 looks like the current best-practice
   stack; `n2p` over iroh is an interesting zero-config LAN/WAN store-sync option.
7. Pin roc-overlay to a dated tag rather than floating `main` once stability matters.

---

*Compiled 2026-08-24. Sources: NixOS blog & release notes (rl-2511/rl-2605),
ryantm.github.io/nixpkgs rl-2611, NixOS/nix release notes 2.30–2.34, lix.systems
blog/about, Mic92/cache-shootout, clan.lol docs & Discourse changelogs,
iroh.computer blog/releases, roc-lang gist/GOTO interview/repos.*
