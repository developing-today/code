#!/usr/bin/env sh
if [ -n "$1" ]; then
  CHARM_DIR="$1"
fi
random() {
  echo $(dd if=/dev/urandom bs=1 count=64 2>/dev/null | xxd -p)
}
if [ -z "$CHARM_DIR" ]; then
  CHARM_DIR=~$USER/code/source/identity/data/charms/$(random)
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
sed -i "s|{{CHARM_DIR}}|$CHARM_DIR|g" ./provider/static/init
sed -i "s|{{CHARM_LINK_URL}}|$CHARM_LINK_URL|g" ./provider/static/init

./identity charm kv set dt.identity.secret.TURSO_HOST "$TURSO_HOST"
./identity charm kv set dt.identity.secret.TURSO_AUTH_TOKEN "$TURSO_AUTH_TOKEN"
./identity charm kv set dt.identity.init <<EOF
#!/usr/bin/env pwsh
cd ~\$USER/code/source/identity
./identity charm kv sync
TURSO_HOST=$(./identity charm kv get dt.identity.secret.TURSO_HOST)
export TURSO_HOST
if [ -z "$TURSO_HOST" ]; then
  echo "TURSO_HOST not set"
  exit 1
fi
TURSO_AUTH_TOKEN=$(./identity charm kv get dt.identity.secret.TURSO_AUTH_TOKEN)
export TURSO_AUTH_TOKEN
if [ -z "$TURSO_AUTH_TOKEN" ]; then
  echo "TURSO_AUTH_TOKEN not set"
  exit 1
fi
./provider.sh "$CHARM_DIR" "$INIT_URL" "$PORT" &
./start-server-all.ps1
EOF

PORT=$PORT CHARM_DIR=$CHARM_DIR ./provider/start.sh
