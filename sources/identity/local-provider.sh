#!/usr/bin/env bash

set -e

echo "+ . ~/.turso.auth"
. ~/.turso.auth
echo "<REDACTED>"

set -x

npx --yes expose-wsl@latest

CHARM_LINK_URL=http://$1:3333/link ./provider.sh
