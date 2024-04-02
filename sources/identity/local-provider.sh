#!/usr/bin/env bash

. ~/.turso.auth

CHARM_LINK_URL=http://$1:3333/link ./provider.sh
