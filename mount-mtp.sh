#!/run/current-system/sw/bin/bash

# MTP Phone Mount Script
# Mounts phone via MTP and makes it accessible to all users

MOUNT_DIR="/tmp/mtp-phone"

echo "Unmounting any existing MTP mounts..."
fusermount -u "$MOUNT_DIR" 2>/dev/null
fusermount -u "/nix/persistent/tmp/mtp-phone" 2>/dev/null

echo "Cleaning and recreating mount directory..."
rm -rf "$MOUNT_DIR"
mkdir -p "$MOUNT_DIR"

echo "Detecting MTP devices..."
mtp-detect >/dev/null 2>&1
if [ $? -ne 0 ]; then
  echo "Error: No MTP device found. Make sure your phone is connected and in MTP mode."
  exit 1
fi

echo "Mounting phone with user permissions..."
simple-mtpfs --device 1 -o allow_other,default_permissions "$MOUNT_DIR"

if [ $? -eq 0 ]; then
  echo "Setting directory permissions..."
  chmod 755 "$MOUNT_DIR"

  echo "Phone successfully mounted at: $MOUNT_DIR"
  echo "Available to all users. Access with:"
  echo "  ls /tmp/mtp-phone"
  echo "  cd /tmp/mtp-phone"
  echo ""
  echo "To unmount: fusermount -u $MOUNT_DIR"
else
  echo "Error: Failed to mount phone."
  exit 1
fi
