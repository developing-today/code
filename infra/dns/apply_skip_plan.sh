#!/usr/bin/env bash

set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

TF_PARALLELISM="${TF_PARALLELISM:-1}"
echo "TF_PARALLELISM: $TF_PARALLELISM"

# if [ -n "${SKIP_APPLY:-}" ]; then
#   echo "skipping tf apply"
#   exit 0
# fi

echo "\$0=$0"
script_name="$0"
while [[ "$script_name" == -* ]]; do
    script_name="${script_name#-}"
done

dir="$(dirname -- "$(which -- "$script_name" 2>/dev/null || realpath -- "$script_name")")"
echo "dir: $dir"

TF_PARALLELISM=$TF_PARALLELISM \
  SKIP_PLAN=1 \
  "$dir/apply.sh"
