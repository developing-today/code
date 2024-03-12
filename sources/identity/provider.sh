#!/usr/bin/env bash
if [ -n "$1" ]; then
  CHARM_DATA_DIR="$1"
fi
random() {
  dd if=/dev/urandom bs=1 count="${1:-16}" 2>/dev/null | xxd -p | tr -d '[:space:]'
}
if [ -z "$CHARM_DATA_DIR" ]; then
  RANDOM_ID=$(random)
  IDENTITY_DIR="$(realpath ~)/code/src/identity"
  CHARM_DATA_DIR="$IDENTITY_DIR/data/charm/link/$RANDOM_ID"
fi
if [ -n "$2" ]; then
  INIT_URL=$2
fi
if [ -z "$INIT_URL" ]; then
  INIT_URL="$(hostname -I | awk '{print $1}')/init"
fi
if [ -n "$3" ]; then
  PORT=$3
fi
if [ -z "$PORT" ]; then
  PORT=3333
fi
if [ -n "$4" ]; then
  TURSO_HOST=$4
fi
if [ -z "$TURSO_HOST" ]; then
  echo "TURSO_HOST not set"
  exit 1
fi
if [ -n "$5" ]; then
  TURSO_AUTH_TOKEN=$5 # really shouldn't do this it's viewable in ps, etc.
fi
if [ -z "$TURSO_AUTH_TOKEN" ]; then
  echo "TURSO_AUTH_TOKEN not set"
  exit 1
fi
if [ -n "$6" ]; then
  CHARM_LINK_URL=$6
fi
if [ -z "$CHARM_LINK_URL" ]; then
  IP=$(hostname -I | awk '{print $1}')
  if [ "$(expr substr "$IP" 1 4)" = "172." ]; then
    IP=$(hostname -I | awk '{print $2}')
    if [ "$(expr substr "$IP" 1 4)" = "172." ]; then
      IP="127.0.0.1"
    fi
  fi
  CHARM_LINK_URL="http://$IP:$PORT/link"
fi
cp -f ./init.template.sh ./provider/static/init
sed -i "s|{{CHARM_DATA_DIR}}|$CHARM_DATA_DIR|g" ./provider/static/init
sed -i "s|{{CHARM_LINK_URL}}|$CHARM_LINK_URL|g" ./provider/static/init

./provider/background.sh &
INIT_URL=$INIT_URL PORT=$PORT CHARM_DATA_DIR=$CHARM_DATA_DIR ./provider/start.sh
