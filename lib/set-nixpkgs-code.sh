#!/usr/bin/env bash
set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

get_script_dir() {
    local script_path full_path
    script_path="$(readlink -f "$0")"
    full_path="$(dirname "$script_path")"
    if [[ "$full_path" == "$HOME" || "$full_path" == "$HOME/"* ]]; then
        echo "${full_path/#$HOME/\~}"
    else
        echo "$full_path"
    fi
}

if [ $# -eq 0 ]; then
    target_dir="${HOME}/nixpkgs"
else
    target_dir="$1"
fi

cp "./code-nixpkgs.sh" "${target_dir}/code.sh"

script_dir=$(get_script_dir)
sed -i "s|~/code|${script_dir}|g" "${target_dir}/code.sh"
