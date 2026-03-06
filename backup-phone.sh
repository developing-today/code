#!/usr/bin/env bash

# Script to backup /tmp/mtp-phone to ~/Documents/backup
# Existing files are skipped (not replaced)

SOURCE="/tmp/mtp-phone"
DEST="$HOME/Documents/2026-01-18_oneplus_dsp_backup"

# Check if source directory exists
if [ ! -d "$SOURCE" ]; then
  echo "Error: Source directory '$SOURCE' does not exist."
  exit 1
fi

# Create destination directory if it doesn't exist
mkdir -p "$DEST"

# Copy files, skipping existing ones
# rsync flags:
# -r: recursive
# -v: verbose
# -h: human-readable sizes
# --progress: show transfer progress per file
# --ignore-existing: skip files that already exist on destination
# --no-perms/owner/group: don't try to preserve attributes (MTP doesn't support them)
rsync -rvh --progress --ignore-existing --no-perms --no-owner --no-group --exclude='Android' "$SOURCE"/ "$DEST"/

echo "Backup complete. Files copied to $DEST"
