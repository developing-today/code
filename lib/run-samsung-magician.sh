#!/usr/bin/env bash
echo "ensuring root"
if [ "$EUID" -ne 0 ]; then
  echo "not root, running as root"
  sudo $0
  exit
else
  echo "root"
fi
cd $(mktemp -d)
wget https://download.semiconductor.samsung.com/resources/software-resources/Samsung_SSD_990_PRO_4B2QJXD7.iso
mkdir ./iso
mount -o loop Samsung_SSD_990_PRO_4B2QJXD7.iso ./iso
gzip -dc iso/initrd | cpio -div --no-absolute-filenames
nix-shell -p unzip
cd root/fumagician
./fumagician
