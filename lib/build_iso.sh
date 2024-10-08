#!/usr/bin/env bash
set -e #-o pipefail

echo "Script Name: $(basename "$0")"
echo "Script: $0"
echo "Script Real Path: $(realpath "$0"))"
echo "Args count: $#"
echo "Args:$(printf "\n\"%q\"" "$@")"

hostname=$(hostname)
bootstrap=false
clean=false
flash=false
force=false
iso_type="unattended-installer_offline"

print_usage() {
  echo "Usage: $0 [hostname] [-bootstrap] [-clean] [-flash]"
  echo "Flags can be provided in any order."
}

process_args() {
  while [[ $# -gt 0 ]]; do
    case $1 in
      -bootstrap)
        bootstrap=true
        shift
        ;;
      -clean)
        clean=true
        shift
        ;;
      -flash)
        flash=true
        shift
        ;;
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
        if [[ -z $custom_hostname ]]; then
          custom_hostname="$1"
        else
          echo "Error: Multiple hostnames provided"
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

if [[ -n $custom_hostname ]]; then
  hostname="$custom_hostname"
  echo "Using provided hostname: $hostname"
else
  echo "Using system hostname: $hostname"
fi

echo "Flags:"
echo "  Bootstrap: $bootstrap"
echo "  Clean: $clean"
echo "  Flash: $flash"
echo "  Force: $force"
echo "  ISO Type: $iso_type"

echo "Checking if running as root"

if [ "$EUID" -ne 0 ]; then
  echo "Re-executing with sudo"
  exec sudo "$0" "$@"
fi

echo "Running as root"

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

if [ "$clean" = true ]; then
  # \/ manual confirmation step within this script to avoid accidental deletion,
  # \/ this script always -force removes iso files when -clean flag is provided.
  echo "Force Removing all ISO files for hostname: $hostname"
  ./lib/remove_iso_files.sh "$hostname" -force
  echo "Force Removing all ISO files completed successfully!"
fi

echo "Adding all untracked files to Git"
git add .
echo "Added all untracked files to Git"

echo "Building ISO for hostname: $hostname"
nix build .#nixosConfigurations.\"${iso_type}_$hostname\".config.system.build.isoImage

if [ $? -ne 0 ]; then
  echo "Error: Nix build ISO failed."
  exit 1
fi

echo "Nix build ISO completed successfully."

if [ "$bootstrap" = true ]; then
  bootstrap_script="${script_dir}/bootstrap_iso.sh"

  if [ -f "$bootstrap_script" ]; then
    echo "Executing bootstrap script: $bootstrap_script"
    chmod +x "$bootstrap_script"
    "$bootstrap_script" "$hostname" # -type

    if [ $? -ne 0 ]; then
      echo "Error: Bootstrap script execution failed."
      exit 1
    fi
else
    echo "Warning: Bootstrap script not found at $bootstrap_script"
  fi
fi

echo "ISO build process completed successfully!"

if [ "$flash" = true ]; then
  echo "Flashing ISO to USB drive"
  flash_force=""
  if [ "$force" = true ]; then
    flash_force="-force"
  fi
  # \/ manual confirmation step within this script to avoid accidental flashing,
  # \/ -force flag can be used to skip this manual confirmation step.
  echo "Flash ISO to USB drive for hostname: $hostname"
  echo "Flash Force: $flash_force"
  ./lib/flash_iso_to_sda.sh "$hostname" $flash_force # -type
  echo "Flash ISO to USB drive completed successfully!"
fi

echo "Script Done: $0"
