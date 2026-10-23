#!/usr/bin/env bash
set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

echo "Making all .sh files executable..."

find . -type f \( -name "*.sh" -o -name ".*.sh" \) -print0 | xargs -0 chmod +x

echo "All .sh files have been made executable."
