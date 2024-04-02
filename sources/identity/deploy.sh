#!/usr/bin/env bash
set -ex
function deploy() {
  cd ~
  rm -rf code
  git clone --depth 1 https://github.com/developing-today/code
  cd code/sources/identity
  chmod -R +x *.sh
  CHARM_LINK_URL="${3:-http}://${1:-localhost}:${2:-3333}/${4:-link}" NO_INSTALL="${NO_INSTALL:-1}" ./init.template.sh
}
if [ "$#" -gt 0 ]; then
  deploy $@
fi
