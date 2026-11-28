#!/usr/bin/env bash
set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail
chmod +x lib/*.sh
for script in lib/*.sh; do
  ln -sf "$script" "$(basename "$script" | sed 's/\.sh$//')"
done
