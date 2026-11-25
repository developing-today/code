# Git worktrees: never under /var/

`/var/folders/.../T/` is the macOS per-user temp directory. **It is swept
periodically by the OS.** A sweep has already destroyed ~3000 tracked files
across several active worktrees in a single session, and it does so silently —
`git status` keeps reporting a clean tree from a stale fsmonitor index while the
files are gone.

The environment block advertises a scratch directory as "pre-approved for
external directory access". On this Mac the nix wrapper now points OpenCode's
TMPDIR at `~/.local/share/opencode/tmp`, so that directory is
`~/.local/share/opencode/tmp/opencode`, and `opencode-tmp-guard.nix` keeps
`/var/folders/.../T/opencode` a symlink to it; some tool descriptions still
name the `/var` path. Either way the approval is about _permissions_, not
_durability_, and the place is for scratch output you are willing to lose.
**Never create a git worktree there.**

## Where worktrees go

Create them under the durable opencode worktree root:

```
~/.local/share/opencode/worktree/<descriptive-name>
```

```bash
git -C <main-clone> worktree add \
  ~/.local/share/opencode/worktree/<descriptive-name> \
  -b <branch> origin/main
```

Pick a name nobody else is using. One worktree per writing agent per task —
do not create several for the same job, and do not work in another agent's.
Read-only agents can share one.

OpenChamber's `session.create` with `worktree` (and its New worktree dialog)
creates worktrees under this root already, as
`~/.local/share/opencode/worktree/<project id>/<name>`; use that when you start
a session for a workstream. When you create a worktree by hand for your own
session, move the session into it (`tools.opencode.session.move` in Code
Mode) so every tool defaults to it.

## Never work in the session's default worktree

The user and other agents share it. Edits there have been silently clobbered,
including a regression test that was only recovered because a reviewer noticed
it missing. Create your own and stay in it.

## Worktrees share one remote

All worktrees of a clone share the parent's `origin`. If that URL is wrong,
**every** worktree is wrong at once, and a single `git remote set-url` on the
parent fixes all of them.

Verify immediately before every push:

```bash
git remote get-url origin
```

If it does not name the repository you are working on, **do not push**. Report
it — a repoint is global corruption, not a local mistake.

## Staging

Never `git add .`. Other sessions may have left unrelated edits in a shared
tree, and unrelated in-flight work has been swept into commits this way. Stage
only your own files by explicit path.

## Forbidden

`git reset --hard`, `git restore`, `git checkout -- <path>`, and `git push
--force` are forbidden unless the user explicitly asks for them by name. When a
worktree is damaged, recover non-destructively — `git checkout-index -a` or
`git archive` from a known-good ref — rather than resetting.
