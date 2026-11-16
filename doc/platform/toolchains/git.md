# Git + GitHub CLI — Setup Cycle

Verified 2026-10-21: git 2.43.0 (system), gh 2.101.0 (nixpkgs, via nix profile), `gh auth status` → logged in to github.com as `dezren39`, HTTPS protocol, scopes include `admin:org`, `gist`, `repo`, `workflow`. (Token value never recorded — see Bitwarden name below.)

## Download
- git: base-image system git (survives recycles; apt installs do NOT).
- gh: `nix profile add nixpkgs#gh` — lands in the single nix profile and is auto-synced to `/usr/local/bin/gh` by `~/tools/nix-profile-sync.sh` (see nix.md). Do not apt-install gh; it would vanish on recycle.

## Install
- gh 2.101.0 was installed 2026-09-30 via nix profile. A stub also exists at `~/tools/bin/git` / dispatcher for version-pinned resolution, but the system git is the everyday binary.

## Gather secrets
- gh credential lives in `~/.config/gh/hosts.yml` (on persistent disk). Backup copy in Bitwarden Secrets Manager under the name **`github-cli.hosts-yml`** (project "developing today").
- SSH key backup in Bitwarden under the name **`ssh.id-ed25519`**; on-disk key is at the standard `~/.ssh/id_ed25519` location. Values are never written to docs, chat, or logs.

## Configure
- `~/.gitconfig` credential helpers (fixed 2026-10-02):
  ```ini
  [credential "https://github.com"]
      helper = !gh auth git-credential
  [credential "https://gist.github.com"]
      helper = !gh auth git-credential
  ```
  **NEVER pin an absolute nix store path in user config.** The previous config pinned `!/home/hatch/nix-persist/store/<hash>-gh-2.101.0/bin/.gh-wrapped ...`; store layout drift killed the wrapper and every push died with "could not read Username". Always use the PATH-resolved `!gh auth git-credential` (gh reads its token from hosts.yml itself). Backup of the broken config: `~/.gitconfig.bak-20261002-1356`.
- Top-level git options (`-C`, `--git-dir`) MUST come **before** the subcommand: `git -C <dir> ls-remote origin HEAD`. Placed after the subcommand they are silently misapplied and the command runs against the CWD repo (hit 2026-10-04: returned the wrong repo's HEAD, convincingly).

## Repo layout (who lives where)
- `/home/hatch` is itself a git repo (dezren39/hatch, PUBLIC) — do NOT run git status/add/commit there for nix-program work; it shows a foreign tree.
- `~/hatch-personal` is the private repo (dezren39/hatch-personal) for notes/TODO/memory. Commit there with `git -C ~/hatch-personal`.
- `~/notes/TODO.md` is a SYMLINK into hatch-personal; `~/workspace` is a real dir in the hatch repo, but `~/workspace/goals`, `~/memory`, `~/logs` are symlinks into hatch-personal.
- Standing rule (Drew, 2026-09-30): review `git status` carefully every time; nothing sneaks into commits.

## Test
```bash
git --version        # expected: git version 2.43.0
gh --version         # expected: gh version 2.101.0
gh auth status       # expected: Logged in to github.com account dezren39, active
git -C ~/git/nix ls-remote origin HEAD   # cross-check against FETCH_HEAD / origin/main log before trusting (see quirks)
```

## Known quirks & what has NOT worked
- **Tool-output fabrication risk (hit repeatedly Oct 2026):** a single `git ls-remote` result once returned a fabricated upstream HEAD, and a follow-up rev-parse "confirmed" it — coherence across calls is NOT proof. For consequential claims (rebuild triggers, watermarks), require multi-source agreement: `ls-remote origin HEAD`, `ls-remote origin refs/heads/main`, `.git/FETCH_HEAD`, `git log origin/main`, `git cat-file -t <sha>`.
- Read-tool staleness on hot-appended files (e.g. TODO.md run logs): verify the live tail with `tail`/`wc -l`/`git log` via exec before concluding where a file ends.
- Absolute nix store paths in any config are forbidden (see Configure) — they rot silently on the next store rebuild.
