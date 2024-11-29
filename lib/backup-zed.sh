#!/usr/bin/env zsh
set -exuo pipefail
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${script_dir}" || exit 1
echo "entered: $script_dir"

cp -f "$HOME/.config/zed/settings.json" ./.zed/settings.json
cp -f "$HOME/.local/share/zed/extensions/index.json" ./.zed/index.json
find "$HOME/.local/share/zed/extensions/installed" -mindepth 1 -maxdepth 1 -exec basename {} \; > ./.zed/installed.txt
