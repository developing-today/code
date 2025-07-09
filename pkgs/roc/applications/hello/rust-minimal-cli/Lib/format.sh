#!/usr/bin/env bash
set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail
shopt -s globstar nullglob
files=(**/*.roc *.roc)
roc format "${files[@]}"
