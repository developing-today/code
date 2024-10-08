#!/usr/bin/env bash
set -e #-o pipefail

echo ""
echo "Script Name: $(basename "$0")"
echo "Script: $0"
echo "Script Real Path: $(realpath "$0"))"
echo "Args count: $#"
echo "Args:$(printf "\n\"%q\"" "$@")"

prefix=""
iso_type="bootstrap_unattended-installer_offline"

print_usage() {
    echo "Usage: $0 [prefix]"
    echo "  prefix: Optional prefix for output ISO file"
}

process_args() {
  while [[ $# -gt 0 ]]; do
    case $1 in
      # -prefix
      -*)
        echo "Error: Unknown flag $1"
        print_usage
        exit 1
        ;;
      # iso_type
      *) # files/dirs to copy recursively
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
echo "  iso_type: $iso_type"

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


count_isos() {
  local pattern="$1"
  ls ${pattern} 2>/dev/null | wc -l
}

iso_dir="./result/iso"
echo "Checking for ISO file in $iso_dir"
iso_pattern="$iso_dir/*.iso"
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

echo "Creating temporary directory for mounting ISO"
mount_point=$(mktemp -d)
echo "Created temporary mount point: $mount_point"

echo "Mounting ISO to $mount_point"
sudo mount -o loop "$iso_file" "$mount_point"
echo "Mounted ISO to $mount_point"

echo "Creating temporary directory for writable ISO contents"
writable_dir=$(mktemp -d)
echo "Created writable directory: $writable_dir"

echo "Copying contents of mounted ISO to writable directory"
rsync -a "$mount_point/" "$writable_dir/"
echo "Copied contents of mounted ISO to writable directory"

echo "Unmounting original ISO"
sudo umount "$mount_point"
echo "Unmounted original ISO"

echo ""

echo "Creating /bootstrap directory"
mkdir -p "$writable_dir/bootstrap"
echo "Created /bootstrap directory"

echo ""
# CUSTOM FILES START

# TODO: allow custom bootstrap files to be copied

echo "Copying /etc/ssh/ssh_host_ed25519_key to /bootstrap"
cp /etc/ssh/ssh_host_ed25519_key "$writable_dir/bootstrap/"
echo "Copied ssh_host_ed25519_key to /bootstrap"

echo "Copying repository root to /bootstrap as tar.gz"
echo "Repository root: $repo_root"
repo_root_basename=$(basename "$repo_root")
echo "Repository root basename: $repo_root_basename"
output_tar_file="$repo_root_basename.tar.gz"
output_tar="$writable_dir/bootstrap/$output_tar_file"
echo "Output tar.gz: $output_tar"
echo "Creating tar.gz of repository root: $repo_root"
tar -czf "$output_tar" -C "$repo_root" .
echo "Created tar.gz of repository root: $repo_root"

# CUSTOM FILES END
echo ""
# TODO remove this?
# echo "Setting permissions 777 for /bootstrap and its contents"
# chmod -R  "$writable_dir/bootstrap"
# echo "Set permissions 777 for /bootstrap and its contents"

echo "Creating new ISO file name with prefix: $prefix"
output_iso="${prefix}${iso_type}_$(basename "$iso_file")"
echo "Output ISO: $output_iso"

echo "Creating new ISO: $output_iso"
echo ""
xorriso -as mkisofs \
  -iso-level 3 \
  -full-iso9660-filenames \
  -volid "nixos-minimal-24.11-x86_64" \
  -eltorito-boot isolinux/isolinux.bin \
  -eltorito-catalog isolinux/boot.cat \
  -no-emul-boot \
  -boot-load-size 4 \
  -boot-info-table \
  -eltorito-alt-boot \
  -e boot/efi.img \
  -no-emul-boot \
  -isohybrid-gpt-basdat \
  -isohybrid-mbr "$writable_dir/isolinux/isohdpfx.bin" \
  -output "$output_iso" \
  "$writable_dir"
echo "Created new ISO: $output_iso"

echo "Cleaning up temporary directories"
rm -rf "$mount_point" "$writable_dir"
echo "Cleaned up temporary directories"

echo "Process completed successfully!"

echo "Script Done: $0"
echo ""
