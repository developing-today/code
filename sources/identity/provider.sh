#!/usr/bin/env bash
set -ex
exec 3>&1 4>&2
trap 'exec 2>&4 1>&3' 0 1 2 3
random() {
  dd if=/dev/urandom bs=1 count="${1:-16}" 2>/dev/null | xxd -p | tr -d '[:space:]'
}
RANDOM_ID=$(random)
exec 1> >(tee -a ./.provider.$RANDOM_ID.$(date +%s).log) 2>&1
if [ -n "$1" ]; then
  CHARM_DATA_DIR="$1"
fi
if [ -z "$CHARM_DATA_DIR" ]; then
  # CHARM_DATA_DIR="./data/charm/link/$RANDOM_ID"
  CHARM_DATA_DIR="./data/charm/provider"
fi
if [ -n "$3" ]; then
  PORT=$3
fi
if [ -z "$PORT" ]; then
  PORT=3333
fi
if [ -n "$2" ]; then
  INIT_URL=$2
fi
IP=$(hostname -I | awk '{print $1}') # this or the hostname method
if [ "$(expr substr "$IP" 1 4)" = "172." ]; then
  IP=$(hostname -I | awk '{print $2}')
  if [ "$(expr substr "$IP" 1 4)" = "172." ]; then
    IP="127.0.0.1"
  fi
fi
if [ -z "$INIT_URL" ]; then
  INIT_URL="http://$IP:$PORT/init"
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
  CHARM_LINK_URL="http://$IP:$PORT/link"
fi
mkdir -p ./provider/static
cp -f ./init.template.sh ./provider/static/init
sed -i "s|{{CHARM_DATA_DIR}}|$CHARM_DATA_DIR|g" ./provider/static/init
sed -i "s|{{CHARM_LINK_URL}}|$CHARM_LINK_URL|g" ./provider/static/init

echo "#!/usr/bin/env bash" > .env

echo "TURSO_HOST=$TURSO_HOST" >> .env
echo "TURSO_AUTH_TOKEN=$TURSO_AUTH_TOKEN" >> .env
echo "CHARM_DATA_DIR=$CHARM_DATA_DIR" >> .env
echo "CHARM_LINK_URL=$CHARM_LINK_URL" >> .env
echo "PORT=$PORT" >> .env
echo "INIT_URL=$INIT_URL" >> .env
echo "IDENTITY_DIR=$IDENTITY_DIR" >> .env
echo "CHARM_LINK_URL=$CHARM_LINK_URL" >> .env
echo "BASE_RANDOM_ID=$RANDOM_ID" >> .env
echo "BASE_CHARM_DATA_DIR=$CHARM_DATA_DIR" >> .env

./provider/start.sh
