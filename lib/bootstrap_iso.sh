#!/usr/bin/env bash
set -e

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
fi

iso_count=$(ls -1 ./result/iso/*.iso 2>/dev/null | wc -l)
if [ "$iso_count" -ne 1 ]; then
    echo "Error: Expected exactly one ISO file in ./result/iso/, found $iso_count"
    exit 1
fi
iso_file=$(ls ./result/iso/*.iso)
echo "Found ISO file: $iso_file"
mount_point=$(mktemp -d)
echo "Created temporary mount point: $mount_point"
sudo mount -o loop "$iso_file" "$mount_point"
echo "Mounted ISO to $mount_point"
writable_dir=$(mktemp -d)
echo "Created writable directory: $writable_dir"
rsync -a "$mount_point/" "$writable_dir/"
sudo umount "$mount_point"
echo "Unmounted original ISO"
mkdir -p "$writable_dir/bootstrap"
echo "Created /bootstrap directory"
# TODO: allow other bootstrap files
cp /etc/ssh/ssh_host_ed25519_key "$writable_dir/bootstrap/"
echo "Copied ssh_host_ed25519_key to /bootstrap"
chmod -R 777 "$writable_dir/bootstrap"
echo "Set permissions 777 for /bootstrap and its contents"
output_iso="${prefix}bootstrapped_$(basename "$iso_file")"
genisoimage -o "$output_iso" -R -J -v -d -N -no-emul-boot -boot-load-size 4 -boot-info-table -b isolinux/isolinux.bin -c isolinux/boot.cat -graft-points "/bootstrap=$writable_dir/bootstrap" "$writable_dir"
echo "Created new ISO: $output_iso"
rm -rf "$mount_point" "$writable_dir"
echo "Cleaned up temporary directories"
echo "Process completed successfully!"
