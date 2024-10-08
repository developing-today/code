#!/usr/bin/env bash
set -e #-o pipefail

echo ""
echo "Script Name: $(basename "$0")"
echo "Script: $0"
echo "Script Real Path: $(realpath "$0"))"
echo "Args count: $#"
echo "Args:$(printf "\n\"%q\"" "$@")"

force=false
prefix=""
iso_type="bootstrap_unattended-installer_offline"

flash_device_alias="SCSI"
flash_device_type="/dev/sd"
flash_device_id="a"
flash_device="${flash_device_type}${flash_device_id}"

expected_root_device_alias="NVMe"
expected_root_device_type="/dev/nvme"

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
      # iso_type
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

echo "Processing args"
process_args "$@"
echo "Processed args"

echo "Using prefix: $prefix"

echo "Flags:"
echo "  force: $force"

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

if [ -n "$prefix" ]; then
  echo "Using prefix: $prefix"
  prefix="${prefix}_"
  echo "Full prefix: $prefix"
else
  echo "No prefix specified."
fi

if [ -n "$prefix" ]; then
  iso_pattern="${prefix}${iso_type}_nixos-*.*.*.*-*-linux.iso"
else
  iso_pattern="*${iso_type}_nixos-*.*.*.*-*-linux.iso"
fi

count_isos() {
  local pattern="$1"
  ls ${pattern} 2>/dev/null | wc -l
}

iso_count=$(count_isos "$iso_pattern")
echo "Found $iso_count ISO files matching pattern: $iso_pattern"
if [ $iso_count -eq 0 ]; then
  echo "Error: No matching ISO file found."
  exit 1
elif [ $iso_count -gt 1 ]; then
  echo "Error: Multiple matching ISO files found. Please specify the host as an argument."
  echo "Found ISO files:"
  ls $iso_pattern
  print_usage
  exit 1
fi

echo "Finding ISO file in $iso_dir"
iso_file=$(ls $iso_pattern)
echo "Found ISO file: $iso_file"

echo ""
echo "$flash_device_alias drives:"
ls "$flash_device_type"*

if ls "$expected_root_device_type"* > /dev/null 2>&1; then
  echo ""
  echo "$expected_root_device_alias drives:"
  ls "$expected_root_device_type"*
  echo ""
  echo "It appears you have an $expected_root_device_alias drive, the $expected_root_device_alias may be your primary boot drive."
else
  echo ""
  echo "You do not have an $expected_root_device_alias drive, $flash_device may be your primary boot drive."
fi

root_drive=$(mount | grep ' / ' | cut -d' ' -f1)
echo ""
echo "Your current root drive is: $root_drive"
echo ""

if [[ $root_drive == "$flash_device"* ]]; then
  echo "WARNING: $flash_device appears to be the root drive!"
  echo "Proceeding may render your system unbootable."
  echo ""
fi

echo "Please verify that $flash_device is the correct drive."
echo ""
echo "The following ISO file will be flashed:"
echo "$iso_file"
echo ""
echo "This will erase all data on: $flash_device"

if [[ "$force" = "true" ]]; then
  echo "Force flag set, skipping confirmation prompt."
else
  read -p "Type 'yes' to continue: " -r
  if [[ $REPLY != "yes" ]]; then
    echo "Aborted"
    exit 1
  fi
fi

if [[ ! -e $flash_device ]]; then
  echo "Error: $flash_device does not exist"
  exit 1
fi

echo "Flashing $iso_file to $flash_device"
dd if="$iso_file" of="$flash_device" bs=64M status=progress conv=fdatasync
echo "Done flashing $iso_file to $flash_device"

echo "Syncing all cached data to disk"
sync
echo "Done syncing all cached data to disk"

echo "Script Done: $0"
echo ""
