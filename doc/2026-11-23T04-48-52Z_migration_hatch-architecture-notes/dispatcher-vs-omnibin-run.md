# Dispatcher vs omnibin's `nix run`

Answers Drew's question: how does what we do (~/tools/dispatcher.sh) differ from
the `nix run` command from omnibin?

## What our dispatcher does

`~/tools/dispatcher.sh` is a per-command resolver, triggered two ways:
- bash `command_not_found_handle` (installed by `~/tools/shell-env.sh`, loaded via
  `BASH_ENV` by the `/bin/bash` wrapper — every exec gets it), and
- pre-created stubs in `~/tools/bin/` (on PATH) so subprocesses and build tools
  find resolved binaries without bash's handler.

Resolution order for `program` / `program@version`:
1. **nix profile fast path** — exact `--version` match against installed profile packages.
2. **Pinned omnibin index** — `nix run <pin-flake>#omnibin -- which --all <name>`,
   querying the prebuilt index DB. No FUSE involved.
3. **Fallback** — `nix run nixpkgs#<name>`.

On a hit it `nix-store --realise`s the binary, adds a **persistent GC root**
(`/nix/var/nix/gcroots/dispatcher/`) so `nix store gc` can't collect it, caches the
resolution on disk (`~/tools/dispatcher/cache/`, ~16ms hits), and creates a stub in
`~/tools/bin/`.

Failure is loud and closed: `cowsay@9.9.9` exits 127 — it never substitutes a
different version. Unknown commands fall through to the normal "command not found".

## What omnibin's `nix run` does

- `nix run github:fzakaria/omnibin` drops you into a **shell** where every nixpkgs
  binary is on PATH, backed by a FUSE daemon + unshare'd shell: binaries
  materialize lazily on first execution.
- `nix run ...#omnibin -- which <name>` is only the index query (no FUSE).
  This is the single piece we use.

## The differences

1. **Granularity.** Ours resolves individual commands inside your normal shell.
   Theirs gives you a whole alternate shell environment to enter.
2. **Persistence.** Ours GC-roots, caches, and stubs every resolved binary:
   instant re-runs, survives `nix store gc`, discoverable by subprocesses.
   Theirs is ephemeral per invocation (re-evaluates the flake, re-fetches).
3. **Version semantics.** Ours implements `program@version` with
   exact-match-or-loud-failure. Plain `nix run nixpkgs#pkg` has no version
   pinning at all.
4. **Reproducibility.** Ours pins the omnibin index as a flake input
   (`~/tools/dispatcher/omnibin/flake.nix` — a wrapper flake with omnibin as an
   input and our `nixpkgs` following omnibin's), refreshed explicitly via
   `~/tools/omnibin-refresh.sh`. `nix run github:fzakaria/omnibin` follows the
   branch.
5. **FUSE.** Ours deliberately never uses it — FUSE is impossible in this
   sandbox (no `/dev/fuse`, mknod denied). Their full-shell mode depends on it.
6. **Failure behavior.** Ours fails closed (exit 127, no substitution).
   `nix run` only fails when the attribute doesn't exist.

## Task D — landed (b2af047)

The pin is now a wrapper flake (`~/tools/dispatcher/omnibin/flake.nix`) taking
omnibin itself as a flake input, with our `nixpkgs` following omnibin's
(`inputs.nixpkgs.follows`). The dispatcher's run command uses the local flake
instead of a `github:` URL. Same index, but our nixpkgs revision matches
omnibin's exactly, so resolved binaries hit their cachix cache instead of
building from source.
