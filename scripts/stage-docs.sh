#!/usr/bin/env bash
# Stage new and modified documentation files, never deletions.
#
# Stages two disjoint sets:
#   1. All untracked (new) files under the target path. Being new, they are by
#      definition neither deleted nor modified. Honours .gitignore.
#   2. All *modified* Markdown files under the target path.
#
# Deletions are never enumerated, so there is no path by which one can be staged.
# This matters for the hardware research tree, where large vendored artifacts are
# routinely moved out to an external archive; those removals must stay unstaged
# for human review rather than being swept into a commit.
#
# Usage:
#   stage-docs.sh [--dry-run] [--all] [PATH]
#
#   --dry-run   Show what would be staged; change nothing.
#   --all       Operate on the whole repository instead of the default paths.
#   PATH        Explicit pathspec to operate on. Overrides --all.
#
# Default scope is doc/hardware plus ai-crawler-site-access-table.md.
#
# Exit codes: 0 success (including "nothing to do"), 1 usage error, 2 not a repo.

set -euo pipefail

# Default scope. More than one entry is allowed: AGENTS.md requires that
# ai-crawler-site-access-table.md be updated alongside hardware research, and it
# lives at the repository root rather than under doc/, so a single-directory
# default would silently never see it.
DEFAULT_PATHS=("doc/hardware" "ai-crawler-site-access-table.md")
dry_run=0
scope=""
explicit_path=""

while [ $# -gt 0 ]; do
  case "$1" in
  --dry-run | -n) dry_run=1 ;;
  --all | -a) scope="all" ;;
  -h | --help)
    sed -n '2,23p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
  -*)
    printf 'error: unknown option %s\n\n' "$1" >&2
    printf 'usage: %s [--dry-run] [--all] [PATH]\n' "${0##*/}" >&2
    exit 1
    ;;
  *)
    if [ -n "$explicit_path" ]; then
      printf 'error: only one PATH may be given (got %s and %s)\n' \
        "$explicit_path" "$1" >&2
      exit 1
    fi
    explicit_path="$1"
    ;;
  esac
  shift
done

# Run from the repository root so pathspecs are unambiguous regardless of cwd.
repo_root="$(git rev-parse --show-toplevel 2>/dev/null)" || {
  printf 'error: not inside a git repository\n' >&2
  exit 2
}
cd "$repo_root"

# Resolve the pathspec. An explicit PATH always wins over --all.
if [ -n "$explicit_path" ]; then
  targets=("$explicit_path")
elif [ "$scope" = "all" ]; then
  targets=()
else
  targets=("${DEFAULT_PATHS[@]}")
fi

if [ "${#targets[@]}" -gt 0 ]; then
  untracked_spec=("${targets[@]}")
  md_spec=()
  for t in "${targets[@]}"; do
    if [ -d "$t" ]; then
      # Git pathspec globs are not path-delimited, so '*' spans '/' and this
      # matches Markdown at any depth beneath the target directory.
      md_spec+=("${t%/}/*.md")
    else
      # A file target is its own pathspec; appending '/*.md' would match nothing.
      md_spec+=("$t")
    fi
  done
  label="${targets[*]}"
else
  untracked_spec=()
  md_spec=("*.md")
  label="<entire repository>"
fi

# Set 1: untracked files. --exclude-standard applies .gitignore et al.
mapfile -d '' -t untracked < <(
  git ls-files --others --exclude-standard -z -- "${untracked_spec[@]+"${untracked_spec[@]}"}"
)

# Set 2: modified Markdown. --diff-filter=M selects modifications only;
# D (deleted) is structurally excluded rather than filtered out afterwards.
mapfile -d '' -t modified_md < <(
  git diff --name-only --diff-filter=M -z -- "${md_spec[@]}"
)

files=("${untracked[@]+"${untracked[@]}"}" "${modified_md[@]+"${modified_md[@]}"}")

printf 'Scope           : %s\n' "$label"
printf 'Untracked (new) : %d\n' "${#untracked[@]}"
printf 'Modified .md    : %d\n' "${#modified_md[@]}"
printf 'Total to stage  : %d\n' "${#files[@]}"

# Report deletions we are deliberately leaving alone, so their absence from the
# staged set is a visible decision rather than a silent omission.
mapfile -d '' -t deleted < <(
  git diff --name-only --diff-filter=D -z -- "${untracked_spec[@]+"${untracked_spec[@]}"}"
)
if [ "${#deleted[@]}" -gt 0 ]; then
  printf 'Deletions left unstaged: %d (not touched by this script)\n' "${#deleted[@]}"
fi

if [ "${#files[@]}" -eq 0 ]; then
  printf '\nNothing to stage.\n'
  # An empty result is ambiguous: it can mean "no changes" or "your changes are
  # outside the scope". Distinguish the two rather than leaving the caller to
  # guess that the default pathspec silently excluded their file.
  if [ "${#targets[@]}" -gt 0 ]; then
    mapfile -d '' -t out_of_scope < <(
      git ls-files --others --exclude-standard -z
      git diff --name-only --diff-filter=M -z -- '*.md'
    )
    if [ "${#out_of_scope[@]}" -gt "${#files[@]}" ]; then
      printf '\nNote: %d candidate file(s) exist elsewhere in the repository but\n' \
        "${#out_of_scope[@]}"
      printf '      fall outside the current scope (%s).\n' "$label"
      printf '      Use --all, or pass an explicit PATH, to include them.\n'
    fi
  fi
  exit 0
fi

# Safety net: every path must exist on disk. If one does not, the working tree
# changed underneath us mid-run and staging could capture a deletion.
missing=0
for f in "${files[@]}"; do
  if [ ! -e "$f" ]; then
    printf 'error: path no longer exists: %s\n' "$f" >&2
    missing=$((missing + 1))
  fi
done
if [ "$missing" -gt 0 ]; then
  printf 'error: %d path(s) vanished mid-run; refusing to stage.\n' "$missing" >&2
  printf '       Re-run - another process is likely modifying the tree.\n' >&2
  exit 1
fi

printf '\n'
if [ "$dry_run" -eq 1 ]; then
  printf 'DRY RUN - nothing staged. Would stage:\n\n'
  printf '  %s\n' "${files[@]}"
  exit 0
fi

printf '%s\0' "${files[@]}" | xargs -0 -r git add --
printf 'Staged %d file(s).\n' "${#files[@]}"
