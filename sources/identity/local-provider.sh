#!/usr/bin/env bash

if [ -z "$TURSO_HOST" ] || [ -z "$TURSO_AUTH_TOKEN" ]; then
  . ~/.turso_auth
fi
if [ -z "$TURSO_HOST" ] || [ -z "$TURSO_AUTH_TOKEN" ]; then
  echo "TURSO_HOST or TURSO_AUTH_TOKEN not set"
  exit 1
fi
CHARM_LINK_URL=http://192.168.2.15:3333/link ./provider.sh
