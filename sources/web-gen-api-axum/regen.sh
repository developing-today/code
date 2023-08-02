#!/bin/bash

set -e
set -x

nix-shell -p tailwindcss --
cargo install tailwindcss-to-rust --version 0.3.2

tailwindcss-to-rust \
    --tailwind-config ./css/tailwind.config.js \
    --input ./css/input.css \
    --output ./src/tailwindcss.rs \
    --rustfmt
tailwindcss --input ./css/input.css --output ./css/output.css
