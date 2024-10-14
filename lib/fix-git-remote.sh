#!/usr/bin/env bash
set -e #-o pipefail

echo ""
echo "Script Name: $(basename "$0")"
echo "Script: $0"
echo "Script Real Path: $(realpath "$0"))"
echo "Args count: $#"
echo "Args:$(printf "\n\"%q\"" "$@")"

force=false
default_remote="origin"
remote=""
repo_url="github.com/developing-today/code"

print_usage() {
    echo "Usage: $0 [remote] [-force]"
    echo "  remote: Optional remote for ISO file matching"
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
        if [[ -z $remote ]]; then
          remote="$1"
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

echo "Processing args"
process_args "$@"
echo "Processed args"

if [[ -z $remote ]]; then
  echo "Using default remote: $default_remote"
  remote="$default_remote"
fi

echo "Using remote: $remote"

echo "Flags:"
echo "  force: $force"

echo "Checking if running as root"

if [ "$EUID" -ne 0 ]; then
  # echo "Re-executing with sudo"
  # exec sudo "$0" "$@"
  echo "Not running as root"
  echo "Ensuring able to sudo"
  sudo whoami
else
  echo "Running as root"
fi

get_script_dir() {
  SOURCE="${BASH_SOURCE[0]}"
  while [ -h "$SOURCE" ]; do
    DIR="$(cd -P "$(dirname "$SOURCE")" && pwd)"
    SOURCE="$(readlink "$SOURCE")"
    [[ $SOURCE != /* ]] && SOURCE="$DIR/$SOURCE"
  done
  echo "$(cd -P "$(dirname "$SOURCE")" && pwd)"
}

find_repo_root() {
  local dir="$1"
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

script_dir=$(get_script_dir)
repo_root=$(find_repo_root "$script_dir")

if [ $? -ne 0 ]; then
  echo "Error: Could not find repository root" >&2
  exit 1
fi

echo "Changing to repository root: $repo_root"
cd "$repo_root"
echo "PWD: $(pwd)"

echo "Getting git remotes"
remotes=$(git remote)
echo "Remotes: $remotes"
remote_exists=false
for r in $remotes; do
  if [[ $r == $remote ]]; then
    echo "Remote $remote exists"
    remote_exists=true
    break
  fi
done
if [[ "$remote_exists" == "false" ]]; then
  echo "Remote $remote does not exist" >&2
else
  echo "git remote remove \"$remote\""
  git remote remove $remote
  echo "done with git remote remove \"$remote\""
fi

echo "git remote add \"$remote\" \"https://\$(sudo cat ~/auth)@$repo_url\""
git remote add "$remote" "https://$(sudo cat ~/auth)@$repo_url"
echo "done with git remote add \"$remote\" \"https://\$(sudo cat ~/auth)@$repo_url\""

echo "Script Done: $0"
echo ""
