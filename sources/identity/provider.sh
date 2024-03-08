#!/usr/bin/env sh
#
# identity provider:
#
# env:
# - TURSO_HOST
# - TURSO_AUTH_TOKEN
#
# params:
# - $1 CHARM_DIR (default: ~$USER/code/source/identity/data/charms/{{random-id}})
# - $2 INIT_URL (default: <ip>:<port>)
# - $3 PORT (default: 3333)
#
# 2. identity provider - run the following series of steps in a loop, possibly in parallel, possibly pre-generate the keys, obtain a port
#  - /init - set link_url and offer script
#    - if get received, return result of consumer script at this address
#  - /link - set charm_url and charm_dir and offer link
#    - generate key, charm kv set
#      - dt.identity.init
#      - dt.identity.secret.TURSO_HOST
#      - dt.identity.secret.TURSO_AUTH_TOKEN
#    - if get received, return result of
#      - charm link (requests time out after 1 minute, obtain the key)
#

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
cd ~`$USER/code/source/identity
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
./provider.sh $CHARM_DIR $INIT_URL $PORT
./start-server-all.ps1
EOF

IP=$(hostname -I | awk '{print $1}')
PORT=3333
CHARM_LINK_URL="http://$IP:$PORT/link"
cp -f ./provider/install.template.sh ./provider/static/link
sed -i "s/{{CHARM_DIR}}/$CHARM_DIR/g" ./provider/static/link
sed -i "s/{{CHARM_LINK_URL}}/$CHARM_LINK_URL/g" ./provider/static/link
. ./provider/start.sh
