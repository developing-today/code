#! /usr/bin/env nix-shell
#! nix-shell -i bash -p bash unzip nvme-cli
set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail
ISO_PREFIX="https://download.semiconductor.samsung.com/resources/software-resources/"
ISO_FILE="Samsung_SSD_990_PRO_4B2QJXD7.iso"
echo "Configured: $ISO_FILE -- see \`<REPO_ROOT>/lib/samsung-firmware.md\` for other models"
ISO_URL="$ISO_PREFIX$ISO_FILE"
echo "ensuring root"
if [ "$EUID" -ne 0 ]; then
  echo "not root, running as root"
  sudo $0
  exit
else
  echo "root user, continuing as"
fi
nvme list
cd $(mktemp -d)
wget "$ISO_URL"
mkdir ./iso
mount -o loop "$ISO_FILE" ./iso
# gzip -dc iso/initrd | cpio -div --no-absolute-filenames
gzip -dc iso/initrd | cpio -di --no-absolute-filenames
cd root/fumagician
./fumagician
nvme list
