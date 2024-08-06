#!/usr/bin/env bash
set -euo pipefail

echo "Making all .sh files executable..."

find . -type f \( -name "*.sh" -o -name ".*.sh" \) -print0 | xargs -0 chmod +x

echo "All .sh files have been made executable."
