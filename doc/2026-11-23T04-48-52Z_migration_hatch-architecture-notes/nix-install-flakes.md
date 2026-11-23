# Installing Nix with Flakes (this sandbox)

How Nix 2.35.2 was installed here, with flakes enabled, in a rootless-hostile
systemd-nspawn container where `/` is an ephemeral overlay.

## The short version

```bash
# 1. Run the full installer (single-user, no daemon)
~/tools/nix-install.sh

# 2. Every boot, the init system verifies it
~/tools/nix-ensure.sh   # ~0.2s when healthy, run by ~/tools/init.sh

# 3. Use it (wrapper handles LD_PRELOAD automatically)
nix --version            # /usr/local/bin/nix wrapper -> real binary
nix run nixpkgs#hello
```

## Step-by-step: what nix-install.sh does

1. **Build users: NOT created** (changed 2026-09-30). `build-users-group =`
   (empty) in `nix.conf` — builds run as the calling UID (root). The old
   `nixbld1`–`nixbld10`/`nixbld` group can't traverse `/home/hatch`
   (`drwxrws--- root:nogroup`), so sandboxed builds failed with Permission
   denied. `nix-ensure.sh`/`nix-install.sh` no longer recreate them; existing
   accounts are inert. Verified: genuine local `builtins.derivation` build
   succeeds.

2. **Downloads the upstream installer** if missing:
   `curl -sSL https://nixos.org/nix/install -o ~/tools/nix-installer.sh`

3. **Runs it single-user**: `sh nix-installer.sh --no-daemon --no-channel-add`
   - `--no-daemon`: no systemd service, no `/nix` daemon socket. The nix
     command talks to the store directly. This is what makes it work inside
     a container without privileged systemd units.
   - `--no-channel-add`: no legacy channels; we use flakes exclusively.

4. **Works around the sandbox taint problem.** The installer's final
   "install into default profile" step fails here because new files get
   `user.hatch_tainted*` xattrs and the sandbox denies `removexattr`.
   The script builds the profile symlinks manually:
   ```
   /nix/var/nix/profiles/per-user/root/profile-1-link -> /nix/store/*-nix-2.*/
   /nix/var/nix/profiles/per-user/root/profile      -> profile-1-link
   /nix/var/nix/profiles/default                     -> .../per-user/root/profile
   ```

5. **Saves a tarball backup**: `~/tools/nix-store.tar.gz` (335 MB, gzip —
   gzip ships in the base image, so restores never depend on apt).
   Never extract over a non-empty tree (tar hardlinks aren't idempotent);
   `nix-ensure.sh` moves a bad tree aside as `.bad.<timestamp>` instead.
   `~/tools/nix-snapshot.sh` re-tars the live tree with `--hard-dereference`.

## Enabling flakes

Flakes are an experimental feature; they must be opted in via `nix.conf`.
`~/tools/nix-ensure.sh` rewrites `/etc/nix/nix.conf` on every boot
(the overlay wipes `/etc`):

```
experimental-features = nix-command flakes
extra-substituter = https://omnibin.cachix.org   # fzakaria's cache
extra-trusted-public-keys = omnibin.cachix.org-1:...  # pinned key
accept-flake-config = true
```

With that in place:

```bash
nix run github:fzakaria/omnibin          # run a flake app
nix run nixpkgs#hello                   # nixpkgs flake shorthand
nix profile install nixpkgs#gh          # install into profile
nix eval --raw nixpkgs#hello.outPath
```

## The /usr/local/bin/nix wrapper

`/etc` and `/usr/local/bin` are wiped on every recycle, so
`~/tools/install-nix-wrapper.sh` (run by `init.sh` on boot) reinstalls it:

```bash
#!/bin/sh
# /usr/local/bin/nix — sets LD_PRELOAD for the taint shim, then execs real nix
export LD_PRELOAD=~/tools/taint-shim/taint_shim.so
exec /nix/var/nix/profiles/default/bin/nix "$@"
```

`~/tools/nix-profile-sync.sh` symlinks `~/.nix-profile/bin/*` into
`/usr/local/bin/` after every `nix profile` mutation, so `nix profile install`ed
apps (e.g. `gh`, `cowsay`) land on PATH with no per-program wrappers.
It skips `nix*` (the wrapper covers those) and never clobbers real files.

## Self-healing

`nix-ensure.sh` also heals a subtle failure mode: a flake-based
`nix profile install` can replace the installer-provided profile generation
and lose the nix binary itself. Since 2026-09-30 the nix binary lives in the
user profile itself (`/nix/var/nix/profiles/per-user/root/profile/bin/nix`,
nix 2.35.2), and the `/usr/local/bin/nix` wrapper execs it directly with a
self-heal: if the profile binary goes missing, `nix-ensure.sh` adds the
newest store nix (with a real, non-symlink `bin/nix`) — never removing
before adding, since `profile remove` + `profile add` of the same path
silently skips linking `bin/nix`.

## On-demand repair

`~/tools/shell-env.sh` (sourced via `BASH_ENV` in every shell) runs a cheap
sentinel: if `/nix`, the profile nix, or `/usr/local/bin/nix` look broken,
it runs `~/tools/nix-repair.sh` synchronously. The repair is `flock`-guarded,
sets `NIX_REPAIR_GUARD` (children inherit; `BASH_ENV` unset; runs via
`/bin/bash.real`) so nested shells never re-trigger, verifies actual state
(`/nix` target, profile nix runs, wrappers match), and never deletes
`/home/hatch/nix-persist` on failure. Full recycle is still handled by
`init.sh` + the `sandbox-boot-init` cron.

## What breaks, and why

| Problem | Cause | Fix |
|---|---|---|
| Installer profile step fails | `user.hatch_tainted*` xattrs, sandbox denies `removexattr` | Manual profile symlinks (in `nix-install.sh`) |
| `nix: the Nix store is not allowed...` symlink error | nix refuses a symlinked `/nix` | `LD_PRELOAD` taint-shim hides the symlink via lstat/stat interposition |
| `nix` missing after `nix profile install` | Flake install replaces profile generation | Profile nix + wrapper self-heal (2026-09-30) |
| Slow first `nix run` after recycle (~2 min) | Store must be re-fetched | `nix-snapshot.sh` tarball restores it with zero download |
| `nix run` wants to build from source | Missing binary cache | omnibin cachix substituter baked into `nix.conf` |
| Sandboxed builds fail (Permission denied) | `nixbld` users can't traverse `/home/hatch` | Traverse-only ACLs for nixbld on `/home/hatch`, nix-persist, taint-shim; `build-users-group = nixbld` (2026-09-30) |
| `allow-symlinked-store` doesn't fully work | Only bypasses the store-open guard; build sandbox + `assertNoSymlinks` still reject symlinked `/nix` | Taint-shim stat/lstat interception kept (also masks mode 0777→0755) |

## References

- Upstream installer: https://nixos.org/nix/install
- Flakes: https://nixos.wiki/wiki/Flakes
- Local scripts: `~/tools/nix-install.sh`, `~/tools/nix-ensure.sh`,
  `~/tools/nix-snapshot.sh`, `~/tools/nix-restore.sh` (legacy shim),
  `~/tools/install-nix-wrapper.sh`, `~/tools/nix-profile-sync.sh`
- Persistence design: [nix-persistence.md](nix-persistence.md)
