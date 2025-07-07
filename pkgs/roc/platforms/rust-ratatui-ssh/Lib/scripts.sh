#!/usr/bin/env bash
set -exuo pipefail
chmod +x lib/*.sh
for script in lib/*.sh; do
  ln -sf "$script" "$(basename "$script" | sed 's/\.sh$//')"
done
