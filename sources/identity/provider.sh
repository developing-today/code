#!/usr/bin/env sh
random() {
  echo $(dd if=/dev/urandom bs=1 count=64 2>/dev/null | xxd -p)
}
if [ -z "$1" ]; then
  CHARM_DIR=~$USER/code/source/identity/data/charms/$(random)
else
  CHARM_DIR=$1
fi
if [ -z "$2" ]; then
  INIT_URL="$(hostname -I | awk '{print $1}')/init"
else
  INIT_URL=$2
fi
if [ -z "$3" ]; then
  PORT=3333
else
  PORT=$3
fi
if [ -z "$TURSO_HOST" ]; then
  echo "TURSO_HOST not set"
  exit 1
fi
if [ -z "$TURSO_AUTH_TOKEN" ]; then
  echo "TURSO_AUTH_TOKEN not set"
  exit 1
fi
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
IP=$(hostname -I | awk '{print $1}')
if [ "$(expr substr "$IP" 1 4)" = "172." ]; then
  IP=$(hostname -I | awk '{print $2}')
  if [ "$(expr substr "$IP" 1 4)" = "172." ]; then
    IP="127.0.0.1"
  fi
fi
PORT=3333
CHARM_LINK_URL="http://$IP:$PORT/link"
cp -f ./init.template.sh ./provider/static/init
sed -i "s|{{CHARM_DIR}}|$CHARM_DIR|g" ./provider/static/init
sed -i "s|{{CHARM_LINK_URL}}|$CHARM_LINK_URL|g" ./provider/static/init
PORT=$PORT CHARM_DIR=$CHARM_DIR ./provider/start.sh
