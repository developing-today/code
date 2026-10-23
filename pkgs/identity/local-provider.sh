#!/usr/bin/env bash

set -Eeuxo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

echo "+ . ~/.turso.auth"
# shellcheck disable=SC1090
. ~/.turso.auth
echo "<REDACTED>"

set -Eeuxo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

npx --yes expose-wsl@latest

CHARM_LINK_URL=http://$1:3333/link ./provider.sh
