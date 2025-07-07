#!/usr/bin/env bash
set -exuo pipefail
shopt -s globstar nullglob
files=(**/*.roc *.roc)
roc format "${files[@]}"
