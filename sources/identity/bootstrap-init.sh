#!/usr/bin/env sh
set -eu
if [ -n "$1" ]; then
  BOOTSTRAP_REPO_NAME="$1"
fi
if [ -z "$BOOTSTRAP_REPO_NAME" ]; then
  BOOTSTRAP_REPO_NAME="code"
fi
if [ -n "$2" ]; then
  BOOTSTRAP_REPO="$2"
fi
if [ -z "$BOOTSTRAP_REPO" ]; then
  BOOTSTRAP_REPO="https://github.com/developing-today/$BOOTSTRAP_REPO_NAME"
fi
if [ -n "$3" ]; then
  BOOTSTRAP_BRANCH="$3"
fi
if [ -z "$BOOTSTRAP_BRANCH" ]; then
  BOOTSTRAP_BRANCH="main"
fi
if [ -n "$4" ]; then
  BOOTSTRAP_DIR="$4"
fi
if [ -z "$BOOTSTRAP_DIR" ]; then
  BOOTSTRAP_DIR="./$BOOTSTRAP_REPO_NAME/src/identity"
fi
if [ -n "$5" ]; then
  BOOTSTRAP_SCRIPT="$5"
fi
if [ -z "$BOOTSTRAP_SCRIPT" ]; then
  # BOOTSTRAP_SCRIPT="bootstrap.sh"
  BOOTSTRAP_SCRIPT="init.template.sh"
fi
if [ -n "$6" ]; then
  BOOTSTRAP_HOST="$6"
fi
if [ -z "$BOOTSTRAP_HOST" ]; then
  BOOTSTRAP_HOST="localhost:3333"
fi
apt update
apt install -y git
git clone "$BOOTSTRAP_REPO"
cd "$BOOTSTRAP_DIR"
chmod +x "./$BOOTSTRAP_SCRIPT"
"./$BOOTSTRAP_SCRIPT" "$BOOTSTRAP_HOST"
