#!/usr/bin/env bash
set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

echo "\$0=$0"
script_name="$0"
while [[ $script_name == -* ]]; do
  script_name="${script_name#-}"
done

dir="$(dirname -- "$(readlink -f -- "$script_name")")"
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
for first_level_dir in "$log_dir"/*/; do
  if [ -d "$first_level_dir" ]; then
    first_level_name=$(basename "$first_level_dir")
    echo "Processing first-level directory: $first_level_name"

    # Iterate over each subdirectory in the first-level directory
    for second_level_dir in "$first_level_dir"/*/; do
      if [ -d "$second_level_dir" ]; then
        second_level_name=$(basename "$second_level_dir")
        echo "Compressing directory: $first_level_name/$second_level_name"

        # Create a tar.gz archive of the second-level subdirectory. Keep the
        # command in the conditional so `set -e` does not skip the failure
        # handler.
        archive_path="${first_level_dir}${second_level_name}.tar.gz"
        if tar -czf "$archive_path" -C "$first_level_dir" "$second_level_name"; then
          echo "Successfully compressed $first_level_name/$second_level_name"

          # Remove the original directory
          rm -rf "$second_level_dir"
          echo "Deleted original directory: $first_level_name/$second_level_name"
        else
          echo "Failed to compress $first_level_name/$second_level_name"
          # A failed tar may leave a partial archive; retain the source and
          # remove only that incomplete output before trying the next folder.
          rm -f "$archive_path"
        fi
      fi
    done
  fi
done

echo "compression of nested log directories completed."
