#!/usr/bin/env bash
set -exuo pipefail
git pull --all
GIT_MERGE_AUTOEDIT=no git merge nixpkgs/master
git push
cd ~/code
./auth.sh
