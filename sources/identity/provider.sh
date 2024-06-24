#!/usr/bin/env bash
set -ex
exec 3>&1 4>&2
trap 'exec 2>&4 1>&3' 0 1 2 3
random() {
  # shellcheck disable=SC2312
  dd if=/dev/urandom bs=1 count="${1:-16}" 2>/dev/null | xxd -p | tr -d '[:space:]'
}
RANDOM_ID=$(
  set -e
  random 16
)
# shellcheck disable=SC2312
exec 1> >(tee -a "./.provider.${RANDOM_ID}.$(date +%s).log") 2>&1
if [[ -n $1 ]]; then
  CHARM_DATA_DIR="$1"
fi
if [[ -z ${CHARM_DATA_DIR} ]]; then
  # CHARM_DATA_DIR="./data/charm/link/$RANDOM_ID"
  CHARM_DATA_DIR="./data/charm/provider"
fi
if [[ -n $3 ]]; then
  PORT=$3
fi
if [[ -z ${PORT} ]]; then
  PORT=3333
fi
if [[ -n $2 ]]; then
  INIT_URL=$2
fi
# shellcheck disable=SC2312
IP=$(hostname -I | awk '{print $1}') # this or the hostname method
# shellcheck disable=SC2312
if [[ ${IP:0:4} == "172." ]]; then
  IP=$(hostname -I | awk '{print $2}')
  if [[ ${IP:0:4} == "172." ]]; then
    IP="127.0.0.1"
  fi
fi
if [[ -z ${IP} ]]; then
  printf "\n=== === === === === ===\nWARNING: IP not found!\n=== === === === === ===\n\n"
fi
if [[ -z ${INIT_URL} ]]; then
  INIT_URL="http://${IP}:${PORT}/init"
fi
if [[ -n $4 ]]; then
  TURSO_HOST=$4
fi
if [[ -z ${TURSO_HOST} ]]; then
  echo "TURSO_HOST not set"
  exit 1
fi
set +x
echo '+ <redacted> [[ -n "$5" ]]'
if [[ -n $5 ]]; then
  echo '+ TURSO_AUTH_TOKEN=$5'
  TURSO_AUTH_TOKEN=$5 # really shouldn't do this it's viewable in ps, etc.
fi
echo '+ <redacted> [[ -z "${TURSO_AUTH_TOKEN}" ]]'
if [[ -z ${TURSO_AUTH_TOKEN} ]]; then
  echo "TURSO_AUTH_TOKEN not set"
  exit 1
fi
echo "+ set -x"
set -x
if [[ -n $6 ]]; then
  CHARM_LINK_URL=$6
fi
if [[ -z ${CHARM_LINK_URL} ]]; then
  CHARM_LINK_URL="http://${IP}:${PORT}/link"
fi

echo "Copying and preparing init.template.sh"
mkdir -p ./provider/static
cp -f ./init.template.sh ./provider/static/init
sed -i "s|{{CHARM_DATA_DIR}}|${CHARM_DATA_DIR}|g" ./provider/static/init
sed -i "s|{{CHARM_LINK_URL}}|${CHARM_LINK_URL}|g" ./provider/static/init
echo "Copy and prepare init.template.sh complete"
echo "Configuring .env"
{
  echo "#!/usr/bin/env bash"
  echo "CHARM_DATA_DIR=${CHARM_DATA_DIR}"
  echo "CHARM_LINK_URL=${CHARM_LINK_URL}"
  echo "PORT=${PORT}"
  echo "INIT_URL=${INIT_URL}"
  echo "CHARM_LINK_URL=${CHARM_LINK_URL}"
  echo "BASE_RANDOM_ID=${RANDOM_ID}"
  echo "BASE_CHARM_DATA_DIR=${CHARM_DATA_DIR}"
  echo "TURSO_HOST=${TURSO_HOST}"
} >.env
set +x
echo '+ <redacted> echo "TURSO_AUTH_TOKEN=${TURSO_AUTH_TOKEN}"'
echo "TURSO_AUTH_TOKEN=${TURSO_AUTH_TOKEN}" >>.env
echo "+ set -x"
set -x
echo "Configure .env complete"

if [[ ! -f ./identity ]]; then
  echo "identity not found"
  echo "running ./build-libsql.sh"
  ./build-libsql.ps1
fi

if ! command -v tcpserver &>/dev/null; then
  sudo apt-get update
  sudo apt-get install -y ucspi-tcp
fi

./provider/start.sh
