#!/usr/bin/env bash
set -ex
function deploy() {
  HOST="${1:-localhost}"
  PORT="${2:-3333}"
  PROTOCOL="${3:-http}"
  HOST_PATH="${4:-link}"
  cd ~
  rm -rf code
  git clone https://github.com/developing-today/code
  cd code/sources/identity
  chmod -R +x *.sh
  CHARM_LINK_URL="${PROTO}://${HOST}:${PORT}/${HOST_PATH}" NO_INSTALL=${NO_INSTALL:-1} ./init.template.sh
}
if [ "$#" -gt 0 ]; then
  deploy $@
fi
