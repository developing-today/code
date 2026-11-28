#!/usr/bin/env bash
if [ -d secrets ]; then
  echo "secrets directory already exists"
  exit 1
fi
sops --decrypt secrets.enc | base64 -d | tar xzf -
