#!/usr/bin/env bash
set -e

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

prefix=""
if [ $# -eq 1 ]; then
  prefix="$1_"
  echo "Using prefix: $prefix"
elif [ $# -gt 1 ]; then
  echo "Error: Too many arguments. Expected 0 or 1 arguments, got $#"
  echo "Usage: $0 [prefix]"
  exit 1
else
  echo "No prefix specified."
fi

count_isos() {
  local pattern="$1"
  ls ${pattern} 2>/dev/null | wc -l
}

if [ -n "$prefix" ]; then
  iso_pattern="${prefix}_bootstrapped_nixos-*.*.*.*-*-linux.iso"
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
  echo "$(ls $iso_pattern)"
  echo "Usage: $0 [prefix]"
  exit 1
fi

iso_file=$(ls $iso_pattern)

echo ""
echo "SCSI drives:"
echo "$(ls /dev/sd*)"
if ls /dev/nvme* > /dev/null 2>&1; then
  echo ""
  echo "NVMe drives:"
  echo "$(ls /dev/nvme*)"
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
  echo "Are you absolutely certain you want to continue??"
  echo ""
fi
echo "Please verify that /dev/sda is the correct drive."
echo ""
echo "The following ISO file will be flashed:"
echo "$iso_file"
echo ""
echo "Are you really sure? This will erase all data on: /dev/sda"
read -p "Type 'yes' to continue: " -r
if [[ $REPLY != "yes" ]]; then
  echo "Aborted"
  exit 1
fi
echo "Flashing $iso_file to /dev/sda"
sudo dd if="$iso_file" of=/dev/sda bs=4M status=progress oflag=sync
echo "Done flashing $iso_file to /dev/sda"
