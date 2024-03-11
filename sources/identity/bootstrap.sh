#!/usr/bin/env sh
if [ -z "$1" ]; then
  if [ -z "$INIT_HOST" ]; then
    INIT_HOST="localhost:3333"
  fi
else
  INIT_HOST="$1"
fi
apt update
apt install -y curl
curl -o ./init "http://$INIT_HOST/static/init"
chmod +x ./init
./init
