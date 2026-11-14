#!/usr/bin/env bash
# Oracle: a cosmetic transformation of the real list must not change JSON.
set -euo pipefail
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

sed 's/^/  /; s/$/ \r/' "$dir/id-pr-checks.txt" >"$tmp/variant"
expected="$(bash "$dir/id-pr-checks-matrix.sh" "$dir/id-pr-checks.txt")"
actual="$(bash "$dir/id-pr-checks-matrix.sh" "$tmp/variant")"
[[ $expected == "$actual" ]]

printf ' alpha \r\n beta\r\n\talpha\t\r\n # comment\r\n \r\n' >"$tmp/duplicates"
[[ "$(bash "$dir/id-pr-checks-matrix.sh" "$tmp/duplicates")" == '["alpha","beta"]' ]]
echo 'id-pr-checks-matrix normalization passed'
