#!/usr/bin/env bash
# Source-snapshot integrity guard for the id workflow (#1569).
#
# Run 40617526037 (2026-10-28) built a torn copy of pkgs/id/src/directory.rs
# that git does not contain: 7 flake checks failed with phantom parse errors
# while the committed file parses and every other workflow on the head
# passed. The divergence point (checkout -> nix flake source copy -> build)
# was never identified, so this guard makes the next occurrence fail fast
# and by name instead of reading as independent code failures:
#
#   snapshot  — at checkout time, record a sha256 manifest of the pkgs/id
#               tree (the git-tracked file set, which is what a flake in a
#               git worktree copies).
#   verify    — after nix is installed, digest the flake source copy in
#               the nix store (the copy the build actually receives) and
#               compare it against the manifest. Any differing, missing,
#               or extra file aborts the job with
#               "source snapshot mismatch" naming the file(s).
#
# Usage:
#   check-id-source-snapshot.sh snapshot <repo-root> <manifest-file>
#   check-id-source-snapshot.sh verify   <repo-root> <manifest-file> <flake-source-dir>/pkgs/id
#
# The verify step's <flake-source-dir> is the pkgs/id subtree of the
# flake source copy. Derive it with getFlake's outPath — the workflow
# previously used `nix flake metadata --json ./pkgs/id | jq -r .path`,
# whose .path is null under Determinate Nix in CI (#1569 wiring fix):
#   nix eval --impure --raw --expr \
#     "(builtins.getFlake \"git+file://$GITHUB_WORKSPACE?dir=pkgs/id\").outPath"
set -euo pipefail

cmd="${1:-}"
root="${2:-}"
manifest="${3:-}"
src="${4:-}"

if [ -z "$cmd" ] || [ -z "$root" ] || [ -z "$manifest" ]; then
  echo "usage: $0 snapshot|verify <repo-root> <manifest-file> [flake-source-dir]" >&2
  exit 2
fi

# Resolve paths before snapshot changes directory, so a relative root is
# interpreted from the caller's working directory, not from pkgs/id.
if ! root="$(cd "$root" 2>/dev/null && pwd -P)"; then
  echo "source snapshot guard: repo root '$root' is not a directory" >&2
  exit 2
fi
manifest_dir="$(dirname -- "$manifest")"
manifest_base="$(basename -- "$manifest")"
if ! manifest_dir="$(cd "$manifest_dir" 2>/dev/null && pwd -P)"; then
  echo "source snapshot guard: manifest directory '$manifest_dir' not found" >&2
  exit 2
fi
manifest="$manifest_dir/$manifest_base"

id_tree="$root/pkgs/id"
if [ ! -d "$id_tree" ]; then
  echo "source snapshot guard: $id_tree not found" >&2
  exit 2
fi

# Emit "sha256  relpath" lines (or "LINK target  relpath" for symlinks)
# for every file in the given tree, sorted by path.
digest_tree() {
  local tree="$1"
  (
    cd "$tree"
    find . -type l -o -type f | LC_ALL=C sort | while IFS= read -r p; do
      rel="${p#./}"
      if [ -L "$p" ]; then
        printf 'LINK %s  %s\n' "$(readlink "$p")" "$rel"
      else
        if ! digest="$(sha256sum -- "$p")"; then
          echo "source snapshot guard: failed to digest $tree/$rel" >&2
          return 1
        fi
        printf '%s\n' "$digest" | sed 's|  \./|  |'
      fi
    done
  )
}

case "$cmd" in
snapshot)
  # The flake source copy contains the git-tracked files of pkgs/id;
  # manifest exactly that set so ignored build residue in a dirty local
  # tree cannot false-positive. In CI the checkout is clean, so this is
  # simply every file git knows about under pkgs/id.
  tmp_manifest="$(mktemp "$manifest.tmp.XXXXXX")" || {
    echo "source snapshot guard: cannot create temporary manifest beside $manifest" >&2
    exit 2
  }
  trap 'rm -f -- "$tmp_manifest"' EXIT
  if git -C "$root" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    (
      cd "$id_tree"
      git -C "$root" ls-files -- pkgs/id | sed 's|^pkgs/id/||' | LC_ALL=C sort |
        while IFS= read -r rel; do
          [ -n "$rel" ] || continue
          if [ ! -e "$rel" ] && [ ! -L "$rel" ]; then
            echo "source snapshot guard: tracked file missing from worktree: $id_tree/$rel" >&2
            exit 1
          fi
          if [ -L "$rel" ]; then
            printf 'LINK %s  %s\n' "$(readlink "$rel")" "$rel"
          else
            if ! sha256sum -- "$rel"; then
              echo "source snapshot guard: failed to digest tracked file $id_tree/$rel" >&2
              exit 1
            fi
          fi
        done
    ) >"$tmp_manifest" || exit 1
  else
    digest_tree "$id_tree" >"$tmp_manifest" || exit 1
  fi
  if [ "$(wc -l <"$tmp_manifest")" -eq 0 ]; then
    echo "source snapshot guard: snapshot recorded no files from $id_tree; refusing an empty manifest" >&2
    exit 1
  fi
  mv -- "$tmp_manifest" "$manifest"
  trap - EXIT
  echo "source snapshot: recorded $(wc -l <"$manifest") files from $id_tree"
  ;;
verify)
  if [ -z "$src" ] || [ ! -d "$src" ]; then
    echo "source snapshot guard: flake source dir '${src}' not found" >&2
    exit 2
  fi
  if [ ! -f "$manifest" ]; then
    echo "source snapshot guard: manifest $manifest missing — run snapshot first" >&2
    exit 2
  fi
  if [ "$(wc -l <"$manifest")" -eq 0 ]; then
    echo "source snapshot guard: manifest $manifest is empty; refusing to verify an empty snapshot" >&2
    exit 2
  fi
  actual="$(mktemp)"
  digest_tree "$src" >"$actual"
  if ! diff -u "$manifest" "$actual" >"$actual.diff"; then
    echo "::error::source snapshot mismatch — the nix build source copy of pkgs/id differs from the checkout snapshot:" >&2
    # Name the differing files plainly (paths from both sides of the diff).
    grep -E '^[+-]' "$actual.diff" | grep -vE '^(\+\+\+|---)' |
      sed -E 's/^[+-]([0-9a-f]{64}|LINK [^ ]*)  /  /' | LC_ALL=C sort -u >&2 || true
    echo "source snapshot mismatch: build copy at $src does not match checkout at $id_tree (see differing files above). This is a CI source-integrity failure (#1569), not a code failure — re-run the job; if it recurs, preserve the manifest and the store path for investigation." >&2
    exit 1
  fi
  echo "source snapshot: build source copy verified against checkout ($(wc -l <"$manifest") files)"
  ;;
*)
  echo "usage: $0 snapshot|verify <repo-root> <manifest-file> [flake-source-dir]" >&2
  exit 2
  ;;
esac
