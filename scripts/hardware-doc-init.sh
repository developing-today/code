#!/usr/bin/env bash
# Ensure the hardware-doc knowledge base is present beside this repository and
# symlinked into place at doc/hardware.
#
#   hardware-doc is a SEPARATE repository: https://github.com/developing-today/hardware-doc
#   It is deliberately not a submodule and not committed here — it is ~440 MB, which
#   would make every clone of this repo roughly 6.5x larger.
#
# Layout produced:
#
#   <repo-parent>/
#   ├── <this repo>/
#   │   └── doc/hardware  ->  ../../hardware-doc     (relative symlink)
#   ├── hardware-doc/                                 (cloned by this script)
#   └── hardware-doc-archive/                         (bulk artifacts; separate repo, usually unpublished)
#
# Safety guarantees:
#   * Never discards uncommitted work. If the checkout is dirty or cannot fast-forward,
#     it warns and leaves everything untouched.
#   * Only ever fast-forwards. No merge, no rebase, no reset, no checkout of tracked files.
#   * Re-runnable. Doing nothing is a success.
set -uo pipefail

REPO_URL="${HARDWARE_DOC_URL:-https://github.com/developing-today/hardware-doc.git}"
DIR_NAME="${HARDWARE_DOC_DIR_NAME:-hardware-doc}"
LINK_REL="doc/hardware"

c_red=$'\033[31m'; c_yel=$'\033[33m'; c_grn=$'\033[32m'; c_dim=$'\033[2m'; c_off=$'\033[0m'
[ -t 1 ] || { c_red=; c_yel=; c_grn=; c_dim=; c_off=; }
info() { printf '%s\n' "$*"; }
ok()   { printf '%s✓%s %s\n' "$c_grn" "$c_off" "$*"; }
warn() { printf '%s!%s %s\n' "$c_yel" "$c_off" "$*" >&2; }
err()  { printf '%sx%s %s\n' "$c_red" "$c_off" "$*" >&2; }

command -v git >/dev/null 2>&1 || { err "git not found in PATH"; exit 1; }

# --- resolve the REAL repository root -----------------------------------------
# --git-common-dir, not --show-toplevel: inside a linked worktree the toplevel is the
# worktree, whose parent is the wrong directory. The common dir always points at the
# main repository's .git, so its parent is the true repo root.
COMMON_DIR="$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" || {
  err "not inside a git repository"; exit 1; }
REPO_ROOT="$(dirname "$COMMON_DIR")"
PARENT="$(dirname "$REPO_ROOT")"
TARGET="$PARENT/$DIR_NAME"

# The symlink must live in *this* working tree, which may be a worktree.
WORKTREE_ROOT="$(git rev-parse --show-toplevel)"
LINK_PATH="$WORKTREE_ROOT/$LINK_REL"

info "${c_dim}repo root : $REPO_ROOT${c_off}"
info "${c_dim}worktree  : $WORKTREE_ROOT${c_off}"
info "${c_dim}target    : $TARGET${c_off}"

# --- clone if absent ----------------------------------------------------------
if [ ! -e "$TARGET" ]; then
  info "cloning $REPO_URL"
  if git clone "$REPO_URL" "$TARGET"; then
    ok "cloned to $TARGET"
  else
    err "clone failed"; exit 1
  fi
elif [ ! -d "$TARGET/.git" ]; then
  err "$TARGET exists but is not a git repository — refusing to touch it"
  exit 1
else
  # --- update, but never destructively ----------------------------------------
  BRANCH="$(git -C "$TARGET" symbolic-ref --quiet --short HEAD 2>/dev/null || true)"

  if [ -z "$BRANCH" ]; then
    warn "$DIR_NAME is in detached HEAD — not updating"
  elif [ -n "$(git -C "$TARGET" status --porcelain 2>/dev/null)" ]; then
    warn "$DIR_NAME has uncommitted changes — not updating"
    git -C "$TARGET" status --short | sed 's/^/      /' >&2
  else
    UPSTREAM="$(git -C "$TARGET" rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null || true)"
    if [ -z "$UPSTREAM" ]; then
      warn "branch '$BRANCH' has no upstream — not updating"
    elif ! git -C "$TARGET" fetch --quiet --prune origin 2>/dev/null; then
      warn "fetch failed (offline?) — leaving $DIR_NAME as-is"
    else
      LOCAL="$(git -C "$TARGET" rev-parse @)"
      REMOTE="$(git -C "$TARGET" rev-parse '@{u}')"
      BASE="$(git -C "$TARGET" merge-base @ '@{u}')"
      if [ "$LOCAL" = "$REMOTE" ]; then
        ok "$DIR_NAME up to date ($BRANCH)"
      elif [ "$LOCAL" = "$BASE" ]; then
        # strictly behind -> fast-forward is safe
        if git -C "$TARGET" merge --ff-only '@{u}' >/dev/null 2>&1; then
          ok "$DIR_NAME fast-forwarded to $(git -C "$TARGET" rev-parse --short @) ($BRANCH)"
        else
          warn "$DIR_NAME fast-forward failed — left unchanged"
        fi
      elif [ "$REMOTE" = "$BASE" ]; then
        warn "$DIR_NAME has unpushed commits on '$BRANCH' — not updating"
      else
        warn "$DIR_NAME has diverged from origin/$BRANCH — resolve manually, not updating"
      fi
    fi
  fi
fi

# --- symlink ------------------------------------------------------------------
# The repo tracks doc/hardware as the relative link ../../hardware-doc, which is correct
# whenever the checkout sits directly beside hardware-doc. It is NOT correct inside a
# linked worktree that lives somewhere else entirely (a common setup), because ../..
# then resolves relative to the worktree's parent, not the real repo's parent.
#
# So: prefer the tracked relative form when it actually resolves to $TARGET; otherwise
# retarget to the absolute path derived from the git common dir, and mark the path
# --skip-worktree so the local deviation does not show up as a modification forever.
# (.gitignore cannot do this - it does not apply to tracked paths.)
REL_DEFAULT="../../$DIR_NAME"

link_resolves_to_target() {
  [ -L "$LINK_PATH" ] || return 1
  [ -d "$LINK_PATH" ] || return 1
  [ "$(cd "$LINK_PATH" && pwd -P)" = "$(cd "$TARGET" && pwd -P)" ]
}

mkdir -p "$(dirname "$LINK_PATH")"

if [ -e "$LINK_PATH" ] && [ ! -L "$LINK_PATH" ]; then
  err "$LINK_PATH exists and is not a symlink - refusing to replace it"
  exit 1
fi

if link_resolves_to_target; then
  ok "$LINK_REL -> $(readlink "$LINK_PATH")"
else
  # try the tracked relative default first, so most clones stay byte-identical to HEAD
  ln -sfn "$REL_DEFAULT" "$LINK_PATH"
  if link_resolves_to_target; then
    ok "$LINK_REL -> $REL_DEFAULT"
  else
    ln -sfn "$TARGET" "$LINK_PATH"
    if link_resolves_to_target; then
      ok "$LINK_REL -> $TARGET  ${c_dim}(absolute: ../.. does not reach the target from here)${c_off}"
    else
      warn "$LINK_REL -> $TARGET  (does not resolve yet)"
    fi
  fi
fi

# Suppress the diff if the link now differs from what HEAD records.
if git -C "$WORKTREE_ROOT" ls-files --error-unmatch "$LINK_REL" >/dev/null 2>&1; then
  if [ -n "$(git -C "$WORKTREE_ROOT" status --porcelain -- "$LINK_REL")" ]; then
    git -C "$WORKTREE_ROOT" update-index --skip-worktree "$LINK_REL" 2>/dev/null \
      && info "${c_dim}  marked --skip-worktree (undo: git update-index --no-skip-worktree $LINK_REL)${c_off}"
  fi
fi

# --- archive (informational) --------------------------------------------------
ARCHIVE="$PARENT/hardware-doc-archive"
if [ -d "$ARCHIVE" ]; then
  ok "archive present: $ARCHIVE ($(du -sh "$ARCHIVE" 2>/dev/null | cut -f1))"
else
  info "${c_dim}archive absent: $ARCHIVE${c_off}"
  info "${c_dim}  bulk artifacts moved out of hardware-doc live there; *.ARCHIVED.md${c_off}"
  info "${c_dim}  placeholders carry hashes and recovery URLs, so it is optional.${c_off}"
fi
