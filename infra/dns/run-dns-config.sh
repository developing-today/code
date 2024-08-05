#!/usr/bin/env bash
set -ex

echo "Building Nix expression..."
result=$(nix-build --no-out-link ./evaluate-dns-config.nix)

if [ -f "$result" ]; then
  echo "Evaluation successful. YAML file generated at: $result"
  echo "Copying file to local directory..."

  rm -f ./dns-config.yaml
  cp "$result" ./dns-config.yaml
  chmod 644 ./dns-config.yaml

  echo "Local file created: dns-config.yaml"
  echo "File contents:"
  cat ./dns-config.yaml
else
  echo "Evaluation failed. No file was produced."
  exit 1
fi
