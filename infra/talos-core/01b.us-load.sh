#!/usr/bin/env bash
if [ -d 01b.us ]; then
  echo "01b.us directory already exists"
  exit 1
fi
sops --decrypt 01b.us.enc | base64 -d | tar xzf -
