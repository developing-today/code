#!/bin/bash
changed_files_file="$(mktemp)"
git diff --name-only --diff-filter=ACMR -z origin/main...HEAD > "$changed_files_file"
mapfile -d '' changed_files < "$changed_files_file"
for f in "${changed_files[@]}"; do
    echo "Changed file: $f"
done
