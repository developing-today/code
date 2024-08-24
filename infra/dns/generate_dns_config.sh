#!/usr/bin/env bash

set -exuo pipefail

echo "\$0=$0"
script_name="$0"
while [[ "$script_name" == -* ]]; do
    script_name="${script_name#-}"
done

dir="$(dirname -- "$(which -- "$script_name" 2>/dev/null || realpath -- "$script_name")")"
echo "script dir: $dir"

output_file="${1:-"$dir/dns_config.yaml"}"
echo "output_file: $output_file"

nix_expression_with_result_file="${2:-"$dir/dns_config/yaml"}"
echo "nix_expression_with_result_file: $nix_expression_with_result_file"

echo "Building Nix expression..."
result=$(nix-build --no-out-link "$nix_expression_with_result_file")

if [ -f "$result" ]; then
  echo "Evaluation successful. YAML file generated at: $result"

  echo "Removing existing local file..."
  rm -f

  echo "Copying file to local directory..."
  cp "$result" "$output_file"

  echo "Setting file permissions..."
  chmod 644 "$output_file"

  echo "Local file created: dns_config.yaml"

  echo "File contents first 25 lines:"
  head -n 25 "$output_file"

  echo "File contents last 25 lines:"
  tail -n 25 "$output_file"
else
  echo "Evaluation failed. No file was produced."
  exit 1
fi
