#!/usr/bin/env sh
if [ -n "$1" ]; then
  BOOTSTRAP_HOST="$1"
fi
if [ -z "$BOOTSTRAP_HOST" ]; then
  BOOTSTRAP_HOST="localhost:3333"
fi
apt update
apt install -y curl
curl -o ./init "http://$BOOTSTRAP_HOST/static/init"
chmod +x ./init
./init
