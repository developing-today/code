#!/usr/bin/env sh
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
  BOOTSTRAP_SCRIPT="bootstrap.sh"
fi
if [ -n "$6" ]; then
  BOOTSTRAP_HOST="$6"
fi
if [ -z "$BOOTSTRAP_HOST" ]; then
  BOOTSTRAP_HOST="localhost:3333"
fi
if [ -n "$7" ]; then
  BOOTSTRAP_PROVIDER_SCRIPT="$7"
fi
if [ -z "$BOOTSTRAP_PROVIDER_SCRIPT" ]; then
  BOOTSTRAP_PROVIDER_SCRIPT="provider.sh"
fi
if [ -n "$8" ]; then
  TURSO_HOST=$8
fi
if [ -z "$TURSO_HOST" ]; then
  echo "TURSO_HOST not set"
  exit 1
fi
if [ -n "$9" ]; then
  TURSO_AUTH_TOKEN=$9 # really shouldn't do this it's viewable in ps, etc.
fi
if [ -z "$TURSO_AUTH_TOKEN" ]; then
  echo "TURSO_AUTH_TOKEN not set"
  exit 1
fi
apt update
apt install -y git
git clone "$BOOTSTRAP_REPO"
cd "$BOOTSTRAP_DIR"
chmod +x "./$BOOTSTRAP_PROVIDER_SCRIPT"
"TURSO_AUTH_TOKEN=$TURSO_AUTH_TOKEN" "TURSO_HOST=$TURSO_HOST" "PORT=3333" "INIT_URL=$BOOTSTRAP_HOST/init" ./"$BOOTSTRAP_PROVIDER_SCRIPT" &
sleep 15
chmod +x "./$BOOTSTRAP_SCRIPT"
source "./$BOOTSTRAP_SCRIPT" "$BOOTSTRAP_HOST"
