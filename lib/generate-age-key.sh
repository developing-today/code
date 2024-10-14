#!/usr/bin/env bash
set -e #-o pipefail

age_key_file="$HOME/.config/sops/age/keys.txt"
echo "age_key_file: $age_key_file"

if [ -f "$age_key_file" ]; then
  echo "age key file exists, are you sure you want to overwrite it?"
  read -p "Are you sure you want to overwrite it? [y/N] " -n 1 -r
  echo
  if [[ $REPLY =~ ^[Yy]$ ]]; then
    echo "Overwriting age key file"
  else
    echo "Aborting"
    exit 1
  fi
fi

echo "Ensuring folders exist"
mkdir -p "$(dirname "$age_key_file")"
echo "folders exist"

echo "Ensuring able to sudo"
sudo whoami

echo "Generating age key"
echo "$(sudo ssh-to-age -private-key -i /etc/ssh/ssh_host_ed25519_key)" > $age_key_file
echo "age key generated"
