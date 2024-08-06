#!/usr/bin/env bash
set -exuo pipefail

dir="$(dirname -- "$(readlink -f -- "$0")")"
echo "dir: $dir"

log_dir="$dir/logs"
echo "log directory: $log_dir"

if [ -f "$dir/.lock" ]; then
  echo "lock file exists, delete it to continue"
  exit 1
fi
touch "$dir/.lock"
function cleanup() {
  echo "cleaning up"
  echo "deleting lock file"
  rm -f "$dir/.lock"
  echo "done cleaning up"
}
trap cleanup EXIT

echo "starting compression of nested log directories..."

# Check if log directory exists
if [ ! -d "$log_dir" ]; then
  echo "Log directory does not exist: $log_dir"
  cleanup
  exit 1
fi

# Iterate over each subdirectory in the log directory
for first_level_dir in "$log_dir"/*/ ; do
  if [ -d "$first_level_dir" ]; then
    first_level_name=$(basename "$first_level_dir")
    echo "Processing first-level directory: $first_level_name"

    # Iterate over each subdirectory in the first-level directory
    for second_level_dir in "$first_level_dir"/*/ ; do
      if [ -d "$second_level_dir" ]; then
        second_level_name=$(basename "$second_level_dir")
        echo "Compressing directory: $first_level_name/$second_level_name"

        # Create a tar.gz archive of the second-level subdirectory
        tar -czf "${first_level_dir}${second_level_name}.tar.gz" -C "$first_level_dir" "$second_level_name"

        # Check if compression was successful
        if [ $? -eq 0 ]; then
          echo "Successfully compressed $first_level_name/$second_level_name"

          # Remove the original directory
          rm -rf "$second_level_dir"
          echo "Deleted original directory: $first_level_name/$second_level_name"
        else
          echo "Failed to compress $first_level_name/$second_level_name"
        fi
      fi
    done
  fi
done

echo "compression of nested log directories completed."
