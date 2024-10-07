#!/usr/bin/env bash
set -e #-o pipefail

force=false
prefix=""

print_usage() {
    echo "Usage: $0 [prefix] [-force]"
    echo "  prefix: Optional prefix for ISO file matching"
    echo "  -force: Skip confirmation prompt"
}

process_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            -force)
                force=true
                shift
                ;;
            -*)
                echo "Error: Unknown flag $1"
                print_usage
                exit 1
                ;;
            *)
                if [[ -z $prefix ]]; then
                    prefix="$1"
                else
                    echo "Error: Too many arguments"
                    print_usage
                    exit 1
                fi
                shift
                ;;
        esac
    done
}

process_args "$@"

# if [ "$EUID" -ne 0 ]; then
#   echo "Re-executing with sudo"
#   exec sudo "$0" "$@"
# fi

find_repo_root() {
  local dir="$PWD"
  while [ "$dir" != "/" ]; do
    if [ -d "$dir/.git" ]; then
      echo "$dir"
      return 0
    fi
    dir="$(dirname "$dir")"
  done
  echo "Error: Not in a Git repository" >&2
  return 1
}

repo_root=$(find_repo_root)
if [ $? -ne 0 ]; then
  echo "Error: Could not find repository root" >&2
  exit 1
fi

echo "Changing to repository root: $repo_root"
cd "$repo_root"

if [ -n "$prefix" ]; then
  prefix="${prefix}_"
  echo "Using prefix: $prefix"
fi

pattern=./${prefix}*.iso
echo "Checking for ISO files in repository root matching pattern: $pattern"
iso_count=$(ls -1 $pattern 2>/dev/null | wc -l)
echo "Found $iso_count ISO files in repository root matching pattern: $pattern"

if [ $iso_count -eq 0 ]; then
  echo "Error: No matching ISO files found in repository root with pattern: $pattern"
  if [ "$force" = true ]; then
    echo "Force flag detected. Exiting without error."
    exit 0
  else
    echo "Aborted"
    exit 1
  fi
fi

echo "$(ls $pattern)"

if [ "$force" = false ]; then
  echo "Are you really sure? This will erase all ISO files in the repository root with pattern: $pattern"
  read -p "[Y/n]: " -n 1 -r
  echo
  if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "Aborted"
    exit 1
  fi
else
  echo "Force flag detected. Proceeding without confirmation."
fi

rm -rf $pattern
echo "ISO files matching pattern $pattern have been removed."
