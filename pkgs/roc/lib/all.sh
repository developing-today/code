#!/usr/bin/env bash
set -Eeuxo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail
./hello roc-lang_rust-basic-cli
./hello go-basic-cli
./display go-basic-cli
./hello rust-basic-cli-template
LINKER=legacy ./hello rust-basic-cli
LINKER=legacy ./hello rust-minimal-cli
echo "Done"
