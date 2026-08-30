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

# --- symlink helper -----------------------------------------------------------
# Both links below are committed in RELATIVE form, because that is correct for the
# normal sibling layout and keeps every clone byte-identical to HEAD. The relative
# form breaks only when a checkout lives somewhere that "../.." does not reach - most
# often a linked worktree parked outside the repo parent. In that case we substitute
# an ABSOLUTE path derived from the git common dir.
#
# Substituting means the worktree now differs from HEAD, so we mark the path
# --skip-worktree to stop it showing as a permanent modification.
#
#   .gitignore CANNOT do this. It only applies to untracked paths; a tracked file
#   keeps reporting changes no matter what .gitignore says. --skip-worktree is the
#   only thing that suppresses it.
#
# CAVEAT, and the reason we print the undo command: while --skip-worktree is set, git
# will refuse to update that path. If the committed link target ever legitimately
# changes upstream, a clone carrying the flag will NOT pick it up on pull, and merges
# or checkouts touching it can fail with "Entry ... not uptodate". Clear it with:
#
#   git update-index --no-skip-worktree <path>
#
# We only ever set the flag when the relative default genuinely does not work here.
#
# ensure_link <repo-workdir> <path-relative-to-that-workdir> <relative-target> <absolute-target>
ensure_link() {
  local wd="$1" rel="$2" reltgt="$3" abstgt="$4"
  local link="$wd/$rel"

  if [ -e "$link" ] && [ ! -L "$link" ]; then
    err "$rel exists and is not a symlink - refusing to replace it"
    return 1
  fi
  mkdir -p "$(dirname "$link")"

  _resolves() { [ -L "$link" ] && [ -e "$link" ] && \
                [ "$(cd "$link" && pwd -P)" = "$(cd "$abstgt" && pwd -P)" ]; }

  if _resolves; then
    ok "$rel -> $(readlink "$link")"
  else
    ln -sfn "$reltgt" "$link"
    if _resolves; then
      ok "$rel -> $reltgt"
    else
      ln -sfn "$abstgt" "$link"
      if _resolves; then
        ok "$rel -> $abstgt  ${c_dim}(absolute: $reltgt does not reach it from here)${c_off}"
      else
        warn "$rel -> $reltgt  (target missing; link left in relative form)"
        ln -sfn "$reltgt" "$link"
        return 0
      fi
    fi
  fi

  # Suppress the diff only if the path is tracked and now differs from HEAD.
  if git -C "$wd" ls-files --error-unmatch "$rel" >/dev/null 2>&1; then
    # Compare WORKTREE against INDEX only. Using `status` here would also fire for a
    # merely-staged-but-uncommitted path, which is not a local deviation at all.
    if ! git -C "$wd" diff --quiet -- "$rel" 2>/dev/null; then
      git -C "$wd" update-index --skip-worktree "$rel" 2>/dev/null \
        && info "${c_dim}  marked --skip-worktree; undo: git -C $wd update-index --no-skip-worktree $rel${c_off}"
    fi
  fi
}

# --- link 1: this repo -> hardware-doc ----------------------------------------
ensure_link "$WORKTREE_ROOT" "$LINK_REL" "../../$DIR_NAME" "$TARGET"

# --- link 2: hardware-doc/archive -> hardware-doc-archive ---------------------
# The archive is a sibling of hardware-doc holding bulk artifacts moved out of it.
# It is its own git repository but is normally unpublished/private because of its
# size, so we only link to it when it already exists locally - we never clone it.
#
#   TO WIRE IN A CLONEABLE ARCHIVE REPO: give it a URL, and mirror the clone/update
#   block used for hardware-doc above, e.g.
#
#     ARCHIVE_URL="${HARDWARE_DOC_ARCHIVE_URL:-}"
#     [ -n "$ARCHIVE_URL" ] && [ ! -e "$ARCHIVE_DIR" ] && git clone "$ARCHIVE_URL" "$ARCHIVE_DIR"
#
#   Keep the same safety rules: fast-forward only, never touch a dirty checkout.
ARCHIVE_DIR="$(dirname "$TARGET")/hardware-doc-archive"

if [ -d "$TARGET/.git" ] || [ -d "$TARGET" ]; then
  if [ -d "$ARCHIVE_DIR" ]; then
    ensure_link "$TARGET" "archive" "../hardware-doc-archive" "$ARCHIVE_DIR"
  else
    info "${c_dim}archive absent: $ARCHIVE_DIR - skipping archive symlink${c_off}"
  fi
fi

# --- archive summary ----------------------------------------------------------
if [ -d "$ARCHIVE_DIR" ]; then
  ok "archive: $ARCHIVE_DIR ($(du -sh "$ARCHIVE_DIR" 2>/dev/null | cut -f1))"
else
  info "${c_dim}archive absent: $ARCHIVE_DIR${c_off}"
  info "${c_dim}  Bulk artifacts moved out of hardware-doc live there. Every one leaves a${c_off}"
  info "${c_dim}  *.ARCHIVED.md placeholder carrying size, SHA-256 and recovery URLs, so the${c_off}"
  info "${c_dim}  archive is optional - its absence costs convenience, not information.${c_off}"
fi
