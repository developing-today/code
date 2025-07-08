#!/usr/bin/env bash
set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail
git pull --all
GIT_MERGE_AUTOEDIT=no git merge nixpkgs/master
git push
cd ~/code
./auth.sh
