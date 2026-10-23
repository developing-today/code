#!/usr/bin/env bash

set -Eeuxo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail
set -Eeuxo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

nix-shell -p tailwindcss --
cargo install tailwindcss-to-rust --version 0.3.2

tailwindcss-to-rust \
  --tailwind-config ./css/tailwind.config.js \
  --input ./css/input.css \
  --output ./src/tailwindcss.rs \
  --rustfmt
tailwindcss --input ./css/input.css --output ./css/output.css
put.css
