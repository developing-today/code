#!/usr/bin/env bash
set -e #-o pipefail

prefix=""

print_usage() {
    echo "Usage: $0 [prefix]"
    echo "  prefix: Optional prefix for output ISO file"
}

process_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
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
  prefix="${prefix}_"
  echo "Using prefix: $prefix"
fi

iso_count=$(ls -1 ./result/iso/*.iso 2>/dev/null | wc -l)
if [ "$iso_count" -ne 1 ]; then
  echo "Error: Expected exactly one ISO file in ./result/iso/, found: $iso_count"
  exit 1
fi

echo "Finding ISO file in ./result/iso/"
iso_file=$(ls ./result/iso/*.iso)
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

echo "Creating /bootstrap directory"
mkdir -p "$writable_dir/bootstrap"
echo "Created /bootstrap directory"

# TODO: allow other bootstrap files
echo "Copying /etc/ssh/ssh_host_ed25519_key to /bootstrap"
cp /etc/ssh/ssh_host_ed25519_key "$writable_dir/bootstrap/"
echo "Copied ssh_host_ed25519_key to /bootstrap"

echo "Setting permissions 777 for /bootstrap and its contents"
chmod -R 777 "$writable_dir/bootstrap"
echo "Set permissions 777 for /bootstrap and its contents"

echo "Creating new ISO file name with prefix: $prefix"
output_iso="${prefix}bootstrapped_$(basename "$iso_file")"
echo "Output ISO: $output_iso"

echo "Creating new ISO: $output_iso"
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
