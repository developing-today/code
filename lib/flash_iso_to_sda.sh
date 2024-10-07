#!/usr/bin/env bash
set -e -o pipefail

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

if [ "$EUID" -ne 0 ]; then
  echo "Re-executing with sudo"
  exec sudo "$0" "$@"
fi

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
  echo "Using prefix: $prefix"
  prefix="${prefix}_"
else
  echo "No prefix specified."
fi

count_isos() {
  local pattern="$1"
  ls ${pattern} 2>/dev/null | wc -l
}

if [ -n "$prefix" ]; then
  iso_pattern="${prefix}bootstrapped_nixos-*.*.*.*-*-linux.iso"
else
  iso_pattern="*bootstrapped_nixos-*.*.*.*-*-linux.iso"
fi

iso_count=$(count_isos "$iso_pattern")
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

iso_file=$(ls $iso_pattern)

echo ""
echo "SCSI drives:"
ls /dev/sd*

if ls /dev/nvme* > /dev/null 2>&1; then
  echo ""
  echo "NVMe drives:"
  ls /dev/nvme*
  echo ""
  echo "It appears you have an NVMe drive, the NVMe may be your primary boot drive."
else
  echo ""
  echo "You do not have an NVMe drive, /dev/sda may be your primary boot drive."
fi

echo ""
echo "Your current root drive is: $(mount | grep ' / ' | cut -d' ' -f1)"
echo ""

root_drive=$(mount | grep ' / ' | cut -d' ' -f1)
if [[ $root_drive == "/dev/sda"* ]]; then
  echo "WARNING: /dev/sda appears to be the root drive!"
  echo "Proceeding may render your system unbootable."
  echo ""
fi

echo "Please verify that /dev/sda is the correct drive."
echo ""
echo "The following ISO file will be flashed:"
echo "$iso_file"
echo ""
echo "This will erase all data on: /dev/sda"

if [ "$force" = true ]; then
  echo "Force flag set, skipping confirmation prompt"
else
  read -p "Type 'yes' to continue: " -r
  if [[ $REPLY != "yes" ]]; then
    echo "Aborted"
    exit 1
  fi
fi

if [[ ! -e /dev/sda ]]; then
  echo "Error: /dev/sda does not exist"
  exit 1
fi

echo "Flashing $iso_file to /dev/sda"
dd if="$iso_file" of=/dev/sda bs=4M status=progress oflag=sync conv=fdatasync
echo "Done flashing $iso_file to /dev/sda"
