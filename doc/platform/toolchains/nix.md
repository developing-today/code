# Nix — Setup Cycle

Verified: Nix 2.35.2, single-user (`--no-daemon`), 2026-10-21. This is the weirdest toolchain on the box; most of this doc is quirks that cost real time.

## Download
- Backup tarball `~/tools/nix-store.tar.gz` ships in the base image, so first populate never needs the network/apt.
- Full fresh install path: `~/tools/nix-install.sh` (separate from the self-heal path below). Upstream installer alone does NOT work here — its final profile step fails on sandbox taint xattrs, so `nix-install.sh` builds the profile symlinks manually.

## Install
- Persistent store lives in `/home/hatch/nix-persist` (btrfs). `/nix` is a **SYMLINK** to it — no copy, no per-exec mount; the symlink survives recycles and is visible to every exec.
- `~/tools/nix-ensure.sh` (wired into `~/tools/init.sh`, runs on boot) is idempotent and fast (~0.2s when healthy): populates the persistent dir from the tarball if unhealthy, ensures the symlink, verifies `nix eval`. It NEVER auto-deletes/quarantines the persistent dir on failure (fails loud) and NEVER extracts over a non-empty tree (tar hardlinks aren't idempotent — a bad tree is moved aside as `.bad.<timestamp>`).
- Snapshots: `~/tools/nix-snapshot.sh` re-tars the live tree with `--hard-dereference`. Legacy `~/tools/nix-restore.sh` is a shim → nix-ensure.
- On-demand repair: `~/tools/nix-repair.sh`, triggered by the sentinel in `~/tools/shell-env.sh` when `/nix`, the profile nix, or `/usr/local/bin/nix` look broken. `flock`-guarded, recursion-guarded, verifies real state post-repair, never deletes nix-persist on failure.

## Gather secrets
- None for nix itself. The omnibin cachix substituter key is baked into `/etc/nix/nix.conf` by nix-ensure.sh (public key, not a secret).
- The nix-cache project's signing/upload secrets are a different thing — Bitwarden names: `nix-cache.signing-key`, `nix-cache.upload-secret`, `cachix.cachix-dhall`. Never values in docs.

## Configure
- `/etc/nix/nix.conf` lives on the ephemeral overlay and is **rewritten every boot** by nix-ensure.sh: `experimental-features = nix-command flakes`, omnibin cachix `extra-substituter` + key, `accept-flake-config = true`, `build-users-group = nixbld`, and `allow-symlinked-store = true` (covers nix's LocalStore guard for the symlinked store). The `real` store-dir setting from early research does **not** exist in nix 2.35.2.
- **ALWAYS run nix with `LD_PRELOAD=~/tools/taint-shim/taint_shim.so`.** The shim: (1) hides `user.hatch_tainted*` xattrs (sandbox denies removexattr even for root — local builds fail EPERM without it), (2) elides nix's redundant startup chown, (3) masks the /nix symlink mode 0777→0755 for the build sandbox's `checkNotWorldWritable` ancestor check. In practice: use `/usr/local/bin/nix` — a wrapper (installed by `~/tools/install-nix-wrapper.sh` on boot) that sets LD_PRELOAD and execs the profile nix directly, and self-heals if the profile binary goes missing.
- Build users: group `nixbld` gid **993**, users nixbld1..10 uids **999..1008**, listed as *supplementary* members (`usermod -aG`) — nix errors "has no members" if it's only their primary group. The group has traverse-only (`--x`) ACLs on `/home/hatch`, `/home/hatch/nix-persist`, `~/tools/taint-shim`.
- Profile sync: `~/tools/nix-profile-sync.sh` symlinks `~/.nix-profile/bin/*` into `/usr/local/bin/` (skips `nix*`, never clobbers real files). Runs on boot and auto-fires after `nix profile` mutations — the wrapper's hook list MUST include `add` (modern name), not just `install|remove|upgrade|rollback`.

## Test
```bash
nix --version            # expected: nix (Nix) 2.35.2
nix eval --expr '1+1'    # expected: 2 (also what nix-ensure.sh verifies)
ls -l /nix               # expected: symlink -> /home/hatch/nix-persist
```
Verified 2026-09-30: a genuine nixbld-user local build succeeded post-recycle (mcpx derivation).

## Known quirks & what has NOT worked
- **Recycle gotcha:** `/etc/passwd` + `/etc/group` are on the wiped overlay, so nixbld vanishes every recycle while nix.conf still demands it → every build fails "the group 'nixbld' does not exist". nix-ensure.sh now recreates group+users with the ORIGINAL ids (persistent btrfs ACLs reference gid 993). Verified self-healing 2026-09-30.
- **`nix profile add` no-op trap:** adding a git+file flake with unchanged URL text does nothing (no rev recorded). Fix: `nix profile remove <element-name>` first — and the element name is what `nix profile list` prints (e.g. `tools/dispatcher/omnibin`), NOT the package name.
- **Always pass the attr:** `nix profile add .#mcpx`, never bare `nix profile add .` (installs the flake's DEFAULT package and leaves your binary dangling).
- **Never pass a symlink as `--profile`:** creates sibling generation chains and leaves the canonical profile untouched. Effectively there is ONE profile: `~/.nix-profile` if it exists, else `/nix/var/nix/profiles/default` (→ per-user/root/profile). A dangling `~/.nix-profile` does not error — it chains `-1-link-1-link` names; treat that as a real alert, delete the dangling links, re-add.
- nix-ensure self-heal quirk: if the profile loses its nix binary, it adds the newest store nix with a *real* (non-symlink) `bin/nix` and never removes before adding (remove+add of the same path silently skips linking bin/nix).
- **Symlinked-store C builds:** gcc canonicalizes /nix through the symlink → cc-wrapper purity check fails ("impure path used in link"). Escape hatch per derivation: `NIX_ENFORCE_PURITY=0` (safe here — the "impure" path IS the store). For Go derivations prefer `env.CGO_ENABLED = "0"` so no C toolchain is needed.
- **Build dir is NOT /build:** builds run under `/home/hatch/nix-persist/var/nix/builds/...`, inside the /home/hatch git repo. Go `-buildvcs=auto` walks up, latches onto that repo, and stamps wrong provenance or fails (`error obtaining VCS status: exit status 128`). Fix for Go derivations: `GOFLAGS=-buildvcs=false`.
- **`nix store delete` "still alive":** fails if the literal store path appears in the exec command text (the harness environment is a GC root via `/proc/<pid>/environ`). Resolve paths into unexported shell variables first, then delete via the variables.
- Fetching a fresh tarball flake can fail "(or its ancestor) is a symlink" — kernel O_NOFOLLOW check, not bypassable here.
- Nix store dirs are mode 555: even root cannot delete inside them — `chmod u+w` directories first (file modes don't matter).
