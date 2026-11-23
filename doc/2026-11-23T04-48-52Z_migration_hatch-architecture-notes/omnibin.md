# omnibin — every nixpkgs binary on your PATH

## What it is

**omnibin** is a FUSE filesystem by Farid Zakaria (fzakaria) that puts
**every binary nixpkgs ever shipped** on your `$PATH`. Nothing is installed,
nothing is built — 0 bytes on disk until something actually reads a file.

> tl;dr: omnibin is a FUSE filesystem that puts every binary nixpkgs ever
> shipped on your `$PATH`. — [fzakaria.com](https://fzakaria.com/2026/09/24/every-package-is-already-installed)

```console
$ nix run github:fzakaria/omnibin
omnibin: tree at /run/user/1000/omnibin, cache at /home/you/.cache/omnibin
$ ls /omnibin/bin | wc -l
51468
$ python3 --version
Python 3.14.6
$ python3@3.6.2 --version
Python 3.6.2
```

Over 50,000 top-level binaries on PATH (the tree holds ~880K versioned
`name@version` forms), spanning nixpkgs builds from 2013 to 2026.

## How it works

1. **File listings crawled from cache.nixos.org.** The NixOS binary cache
   has never been garbage-collected — it's an S3 bucket that only grows.
   omnibin indexes the file listings of every store path.
2. **Index built per release.** Each GitHub release ships a prebuilt index
   (`data-pins.json` in the repo pins the tag + hash of every asset; the
   flake fetches them). Recent cut (`data-20260927`): 51,503 executables on
   PATH for `x86_64-linux`, 882,815 `name@version` forms, 30.5 TB of unpacked
   bytes addressable, dates 2012-07-05 to 2026-09-25.
3. **FUSE + lazy fetch.** Mount the FUSE filesystem; binaries resolve on
   demand and are fetched from the binary cache only when read. The shell
   app (`nix run github:fzakaria/omnibin`) drops you into a shell with the
   whole tree on PATH, backed by an unshare'd mount namespace.

Related projects by the same author:
- **nixpkgs-multiverse** — the "fast mode" index behind omnibin; every
  indexed package version addressable as a fake derivation.
  As of Aug 2026: all 271,187 indexed paths still alive on cache.nixos.org.
- **omniflake** — thousands of Nix flakes behind one flake input
  (`github:fzakaria/omniflake`, site: https://omniflake.com/).
- **trynix** (https://trynix.dev) — boot any nix store path in the browser.

## Links

- Flake / repo: https://github.com/fzakaria/omnibin
  (`nix run github:fzakaria/omnibin`)
- Releases (index cuts): https://github.com/fzakaria/omnibin/releases
- Announcement post: https://fzakaria.com/2026/09/24/every-package-is-already-installed
- Author's blog: https://fzakaria.com
- nixpkgs-multiverse fast mode: https://github.com/fzakaria/fzakaria.com/blob/HEAD/_posts/2026-08-14-nixpkgs-multiverse-fast-mode.md
- "Nix needs relocatable binaries" (why the store path is fixed):
  https://github.com/fzakaria/fzakaria.com/blob/HEAD/_posts/2026-06-21-nix-needs-relocatable-binaries.md
- "One flake to rule them all" (omniflake): https://github.com/fzakaria/fzakaria.com/blob/HEAD/_posts/2026-08-28-one-flake-to-rule-them-all.md
- Docker image: `fmzakari/omnibin` on Docker Hub (needs `--device /dev/fuse --cap-add SYS_ADMIN`)
- cachix cache: `omnibin.cachix.org` (baked into our `/etc/nix/nix.conf`)

## How we use it here (no FUSE)

FUSE is impossible in this sandbox (see [capabilities.md](https://github.com/dezren39/hatch/blob/main/notes/architecture/capabilities.md):
kernel has FUSE, but `mknod` is denied so `/dev/fuse` can't be created).
So we use omnibin **nix-native, no FUSE**:

```bash
# Query the pinned index DB — no mount, no daemon
nix run github:fzakaria/omnibin#omnibin -- which <name>
nix run github:fzakaria/omnibin#omnibin -- which --all <name>   # all versions

# Realise (fetch) any resolved store path and run it
nix-store --realise <store-path>
```

Always run with the taint shim:
`LD_PRELOAD=~/tools/taint-shim/taint_shim.so` (the `/usr/local/bin/nix`
wrapper does this automatically).

### Zero-step `@version` support — the dispatcher

`~/tools/dispatcher.sh` is the single resolver for `program[@version]`,
called by both `command_not_found_handle` (via `BASH_ENV` →
`~/tools/shell-env.sh`) and by executable stubs in `~/tools/bin/` (for
subprocess/build-tool discovery). Type any nixpkgs binary and it resolves,
fetches, and runs:

```bash
$ cowsay@3.8.4 "hello"     # works with zero per-exec setup
```

What it does: resolves name[@version] → downloads the closure
(`nix store realise`) → registers a **persistent GC root** in
`/nix/var/nix/gcroots/dispatcher/` (survives `nix store gc`) → caches the
resolution in `~/tools/dispatcher/cache/` (cache hit = no nix invocation,
~16ms) → creates a stub in `~/tools/bin/<name>` → execs the binary
(preserving args and exit status).

Rules: the omnibin flake is **pinned** (`~/tools/dispatcher/omnibin-pin`,
currently `459dcff6ee82dd1fb7f3706c4ec5519019635a0a`); an explicit
`name@version` with no exact match in the index is a **loud failure**
(exit 127, never a silent substitution). The pin is refreshed ONLY by
`~/tools/omnibin-refresh.sh` (explicit maintenance), never on a miss.
Lookup order: user profile fast path (exact `--version` match) → pinned
omnibin index (`which --all` for `@version`) → fallback `nix run nixpkgs#`.
Misses print `command not found` with exit 127.

Verified 2026-09-30: `cowsay@3.8.4` (profile fast path), `figlet`
(unversioned, omnibin index), stub from `bash -c`, args + exit-status
preservation, survival across `nix store gc` (265 paths deleted, figlet
protected by its GC root), loud failure on `cowsay@9.9.9` (exit 127).
Cache hit: 16ms.

At Drewry's direction: no run script, no `~/tools` copy of omnibin itself —
nix-native only, via the pinned flake.

## What doesn't work here

| Feature | Status | Why |
|---|---|---|
| `nix run github:fzakaria/omnibin` shell app | ❌ fails cleanly | Needs FUSE mount; `/dev/fuse` can't be created (`mknod` EPERM) |
| Index query (`#omnibin -- which`) | ✅ works | No FUSE needed |
| `nix-store --realise` + run | ✅ works | Plain store fetch |
| dispatcher `@version` | ✅ works | Built on the above, with GC roots + cache |
| Docker image | ❌ n/a | No Docker here; also needs `/dev/fuse` + `SYS_ADMIN` |

Needs the runtime to expose `/dev/fuse` for the full experience.
Dev note filed with the Muse team 2026-09-30 (confirmed delivered).
