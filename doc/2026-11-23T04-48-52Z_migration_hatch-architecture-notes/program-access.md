# Program Access

How every program is available in every exec with zero per-exec setup.

## The Challenge

Each exec is a fresh, non-interactive, non-login bash (`/bin/bash` by absolute path):
- No `~/.bashrc`, no `~/.profile` — nothing is sourced
- No environment carries over between execs
- Default PATH includes `/usr/local/bin` but nothing else custom

## Layer 1: `/usr/local/bin/nix` Wrapper

`/usr/local/bin/nix` is a wrapper script (on the default PATH) that sets `LD_PRELOAD=~/tools/taint-shim/taint_shim.so` and execs the real nix binary. Created by `~/tools/install-nix-wrapper.sh`, recreated on boot via `init.sh`.

Result: `nix` works in any exec with no setup.

## Layer 2: Profile Sync

`~/tools/nix-profile-sync.sh` symlinks everything in `~/.nix-profile/bin/` into `/usr/local/bin/`.

- Runs on boot via `init.sh`
- Auto-triggered by the nix wrapper after `nix profile install/remove/upgrade/rollback`
- Skips `nix*` (the wrapper covers those), never clobbers real files

Result: `nix profile install nixpkgs#cowsay` → `cowsay` is immediately on PATH.

## Layer 3: `/bin/bash` Wrapper

`/bin/bash` is replaced with a wrapper script that sets `BASH_ENV=~/tools/shell-env.sh` and execs `/bin/bash.real` (a real bash binary).

- `BASH_ENV` is inherited across exec and auto-sourced by non-interactive bash
- `shell-env.sh` is startup-clean by design: PATH setup + `command_not_found_handle` only, no network/nix (4–5ms startup, audited 2026-09-30)
- Installed by `~/tools/install-bash-wrapper.sh`, recreated on boot via `init.sh`
- Atomic install (temp in /bin + rename); never overwrites `/bin/bash.real` when `/bin/bash` is already a wrapper (verifies ELF first — this exact failure happened 2026-09-30, recovered via nix store bash)
- Rollback: `cp /bin/bash.real /bin/bash`
- Transparent for all other commands

## Layer 4: dispatcher (`@version` + stubs)

`command_not_found_handle` (defined in `shell-env.sh`) execs `~/tools/dispatcher.sh` — the single resolver for `program[@version]`. Stubs in `~/tools/bin/` (on PATH) cover subprocess/build-tool discovery.

1. **Nix profile** (fast path) — exact `--version` match for `name@version`
2. **Pinned omnibin index** — `which --all` for the version (pin in `~/tools/dispatcher/omnibin-pin`; refreshed only by `omnibin-refresh.sh`)
3. **Nixpkgs fallback** — `nix run nixpkgs#program`

Resolved binaries get a persistent GC root (`/nix/var/nix/gcroots/dispatcher/`), disk cache (`~/tools/dispatcher/cache/`, ~16ms hits), and a stub. Explicit `name@version` with no exact match fails loudly (exit 127, never substitutes).

Verified 2026-09-30: `cowsay@3.8.4` (profile), `figlet` (index), stub from `bash -c`, args/exit-status preserved, GC survival.

## Layer 5: on-demand repair

`shell-env.sh` runs a cheap sentinel each startup (stat checks, no nix): if `/nix`, the profile nix, or `/usr/local/bin/nix` look broken, it runs `~/tools/nix-repair.sh` synchronously. Repair uses `flock`, sets `NIX_REPAIR_GUARD` for children (with `BASH_ENV` unset, via `/bin/bash.real`) so nested shells never re-trigger, verifies actual state post-repair, and never deletes `/home/hatch/nix-persist` on failure. Full recycle is still handled by `init.sh` + the `sandbox-boot-init` cron.

## Summary

| What you type | How it resolves |
|---|---|
| `nix ...` | `/usr/local/bin/nix` wrapper → LD_PRELOAD → profile nix (self-healing) |
| `cowsay` | `/usr/local/bin/cowsay` → symlink → nix profile |
| `jq@1.7.1` | `command_not_found_handle` → dispatcher → pinned omnibin → runs |
| `figlet` | `~/tools/bin/figlet` stub → dispatcher (cached) → runs |
| `gh` | `/usr/local/bin/gh` → symlink → nix profile (gh 2.101.0) |
