#!/usr/bin/env bash
set -e #-o pipefail

echo "postInstall starting"
echo "Copying /iso/bootstrap to /mnt/bootstrap..."
cp -r /iso/bootstrap /mnt
echo "Done copying /iso/bootstrap to /mnt/bootstrap"

echo "Listing /mnt/bootstrap..."
ls -lahR /mnt/bootstrap
echo "Done listing /mnt/bootstrap"

echo "Uncompressing all .tar.gz files in /mnt/bootstrap..."
find /mnt/bootstrap -name "*.tar.gz" -exec sh -c '
    dir=$(dirname "$1")
    base=$(basename "$1" .tar.gz)
    mkdir -p "$dir/$base"
    tar -xzf "$1" -C "$dir/$base"
' sh {} \;
echo "Done uncompressing all .tar.gz files in /mnt/bootstrap"

echo "postInstall done"
