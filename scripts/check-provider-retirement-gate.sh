#!/usr/bin/env bash
# Retirement gate for local proxies and model providers (#5610).
#
# When retiring a provider door, systemd unit, or proxy component,
# this gate verifies that no active references to that provider ID
# remain in OpenCode configuration files (repo tracked config,
# user deployed config, and runtime state configs).
#
# Usage:
#   scripts/check-provider-retirement-gate.sh [options] <provider-id> [provider-id...]
#
# Options:
#   --config <path>   Explicit config file to check (may be repeated).
#   --allow-empty     Succeed even if no config files exist on disk.
#   -h, --help        Show this help message.
#
# Exit codes:
#   0: No references found across checked configs (safe to retire)
#   1: Active references exist (retirement BLOCKED)
#   2: Usage error / missing files
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/check-provider-retirement-gate.sh [options] <provider-id> [provider-id...]

Check that no configuration references remain for a provider before retiring its
service unit, proxy script, or network port (#5610).

Options:
  --config <path>   Explicit config file to check (can be specified multiple times).
  --allow-empty     Exit 0 if no config files exist on disk.
  -h, --help        Show this help message.

Exit codes:
  0: Safe to retire — zero references found across all checked config files.
  1: Retirement BLOCKED — active references found.
  2: Configuration error or invalid usage.
EOF
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

configs=()
providers=()
allow_empty=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --config)
      if [[ $# -lt 2 ]]; then
        echo "::error::--config requires a file path." >&2
        exit 2
      fi
      configs+=("$2")
      shift 2
      ;;
    --allow-empty)
      allow_empty=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      echo "::error::Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      providers+=("$1")
      shift
      ;;
  esac
done

if [[ ${#providers[@]} -eq 0 ]]; then
  echo "::error::At least one provider ID must be specified." >&2
  usage >&2
  exit 2
fi

# Resolve candidate configs if none explicitly specified
resolved_configs=()
if [[ ${#configs[@]} -gt 0 ]]; then
  for cfg in "${configs[@]}"; do
    if [[ ! -f "$cfg" ]]; then
      echo "::error::Specified config file does not exist: $cfg" >&2
      exit 2
    fi
    resolved_configs+=("$cfg")
  done
else
  # Default paths checked for OpenCode configurations
  default_candidates=(
    "$repo_root/.opencode/opencode.jsonc"
    "${XDG_CONFIG_HOME:-$HOME/.config}/opencode/opencode.jsonc"
    "${XDG_DATA_HOME:-$HOME/.local/share}/opencode/opencode.jsonc"
    "${XDG_STATE_HOME:-$HOME/.local/state}/opencode/opencode-runtime.jsonc"
  )
  for candidate in "${default_candidates[@]}"; do
    if [[ -f "$candidate" ]]; then
      resolved_configs+=("$candidate")
    fi
  done
fi

if [[ ${#resolved_configs[@]} -eq 0 ]]; then
  if [[ $allow_empty -eq 1 ]]; then
    echo "Retirement gate OK: no configuration files found to check (--allow-empty)."
    exit 0
  fi
  echo "::error::No OpenCode configuration files found to check." >&2
  exit 2
fi

total_violations=0

for provider in "${providers[@]}"; do
  provider_matches=0
  for cfg in "${resolved_configs[@]}"; do
    if grep -qF "$provider" "$cfg"; then
      matches="$(grep -nF "$provider" "$cfg" || true)"
      count="$(echo "$matches" | wc -l)"
      echo "::error::Config file '$cfg' contains $count reference(s) to provider '$provider':" >&2
      while IFS= read -r line; do
        echo "  $line" >&2
      done <<<"$matches"
      provider_matches=$((provider_matches + count))
    fi
  done

  if [[ $provider_matches -gt 0 ]]; then
    echo "::error::Provider '$provider' has $provider_matches active reference(s) across configuration files. Retirement BLOCKED (#5610)." >&2
    total_violations=$((total_violations + provider_matches))
  else
    echo "Retirement gate OK: no references to provider '$provider' found across ${#resolved_configs[@]} config file(s)."
  fi
done

if [[ $total_violations -gt 0 ]]; then
  exit 1
fi

exit 0
