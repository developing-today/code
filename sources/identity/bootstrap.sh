#!/usr/bin/env sh
if [ -n "$1" ]; then
  BOOTSTRAP_HOST="$1"
fi
if [ -z "$BOOTSTRAP_HOST" ]; then
  BOOTSTRAP_HOST="localhost:3333"
fi
if [ -n "$2" ]; then
  BOOTSTRAP_URL_PATH="$2"
fi
if [ -z "$BOOTSTRAP_URL_PATH" ]; then
  BOOTSTRAP_URL_PATH="static/init"
fi
if [ -n "$3" ]; then
  BOOTSTRAP_SCRIPT="$3"
fi
if [ -z "$BOOTSTRAP_SCRIPT" ]; then
  BOOTSTRAP_SCRIPT="init"
fi
apt update
apt install -y curl
curl -o "$BOOTSTRAP_SCRIPT" "http://$BOOTSTRAP_HOST/$BOOTSTRAP_URL_PATH/$BOOTSTRAP_SCRIPT"
chmod +x "./$BOOTSTRAP_SCRIPT"
"./$BOOTSTRAP_SCRIPT"
