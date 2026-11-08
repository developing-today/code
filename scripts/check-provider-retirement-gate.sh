#!/usr/bin/env bash
# Retirement gate for local proxies and model providers (#5610).
#
# When retiring a provider door, systemd unit, or proxy component,
# this gate verifies that no active references to that provider ID
# remain in OpenCode configuration files (repo tracked config,
# user deployed config, and runtime state configs).
#
# "Active" means a configuration value: JSONC comments (// line and
# /* */ block) are stripped before searching, so retirement history
# recorded in comments does not block a retirement whose real
# references are gone (PR #7679 Codex finding 4506184819).
#
# Matching is by provider identity, not arbitrary substring: a
# reference is a quoted provider key ("<id>": ...) or a model
# reference ("<id>/<model>"). An ID that is merely a substring of an
# unrelated provider, package, or model name (retiring "opencode"
# while "opencode-route/workhorse" exists) does not block
# (PR #7679 Codex finding 4506371633).
#
# Entries in a "disabled_providers" array are tombstones, not active
# references: the tracked config REQUIRES that already-retired
# providers stay listed there, because removing an entry reactivates
# the provider through the catalog. Those entries are blanked before
# searching (PR #7679 Codex finding 4506484591).
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
#   2: Usage error / missing files / a config that could not be searched
#      (fail closed: an unreadable config is never "no references")
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
  2: Configuration error or invalid usage (including a config file that
     could not be read/searched — the gate fails closed).
EOF
}

# Strip JSONC comments from a config file, writing the result to stdout.
# Comment characters are replaced with spaces (newlines preserved) so
# line numbers in the output still match the source file. String
# literals are passed through untouched, including // and /* sequences
# inside them. Returns non-zero when the file cannot be read.
strip_jsonc_comments() {
  awk '
    BEGIN { in_str = 0; in_block = 0; esc = 0 }
    {
      out = ""
      n = length($0)
      for (i = 1; i <= n; i++) {
        c = substr($0, i, 1); d = substr($0, i + 1, 1)
        if (in_block) {
          if (c == "*" && d == "/") { in_block = 0; out = out "  "; i++ }
          else { out = out " " }
          continue
        }
        if (in_str) {
          out = out c
          if (esc) { esc = 0 }
          else if (c == "\\") { esc = 1 }
          else if (c == "\"") { in_str = 0 }
          continue
        }
        if (c == "\"") { in_str = 1; out = out c; continue }
        if (c == "/" && d == "/") {
          for (j = i; j <= n; j++) out = out " "
          i = n
          break
        }
        if (c == "/" && d == "*") { in_block = 1; out = out "  "; i++; continue }
        out = out c
      }
      print out
    }' "$1"
}

# Blank the entries of any "disabled_providers" array in a
# comment-stripped config, preserving line numbers: a tombstone entry
# suppresses a retired provider, it does not use it.
strip_disabled_providers() {
  awk '
    BEGIN { in_str = 0; esc = 0; awaiting_colon = 0; awaiting_bracket = 0; in_disabled = 0; depth = 0; str = "" }
    {
      out = ""
      n = length($0)
      for (i = 1; i <= n; i++) {
        c = substr($0, i, 1)
        if (in_disabled) {
          out = out " "
          if (in_str) {
            if (esc) { esc = 0 }
            else if (c == "\\") { esc = 1 }
            else if (c == "\"") { in_str = 0 }
            continue
          }
          if (c == "\"") { in_str = 1; esc = 0; continue }
          if (c == "[") { depth++; continue }
          if (c == "]") { depth--; if (depth == 0) { in_disabled = 0 } continue }
          continue
        }
        if (awaiting_colon) {
          if (c == " " || c == "\t") { out = out c; continue }
          if (c == ":") { awaiting_colon = 0; awaiting_bracket = 1; out = out c; continue }
          awaiting_colon = 0
        }
        if (awaiting_bracket) {
          if (c == " " || c == "\t") { out = out c; continue }
          if (c == "[") { awaiting_bracket = 0; in_disabled = 1; depth = 1; out = out " "; continue }
          awaiting_bracket = 0
        }
        if (in_str) {
          out = out c
          if (esc) { esc = 0 }
          else if (c == "\\") { esc = 1 }
          else if (c == "\"") { in_str = 0; if (str == "disabled_providers") { awaiting_colon = 1 } str = "" }
          else { str = str c }
          continue
        }
        if (c == "\"") { in_str = 1; esc = 0; str = ""; out = out c; continue }
        out = out c
      }
      print out
    }' "$1"
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
    # The deployed standing server's runtime overlay lives at a FIXED
    # location (home/common/default.nix sets OPENCODE_CONFIG to
    # %h/.local/state/opencode/opencode-runtime.jsonc) regardless of
    # XDG_STATE_HOME, so always check that exact file too — an XDG
    # override must not steer the gate away from the deployed overlay
    # (PR #7679 Codex finding 4506371638).
    "$HOME/.local/state/opencode/opencode-runtime.jsonc"
  )
  for candidate in "${default_candidates[@]}"; do
    if [[ -f "$candidate" ]]; then
      already=0
      for existing in "${resolved_configs[@]}"; do
        if [[ "$existing" == "$candidate" ]]; then already=1; break; fi
      done
      if [[ $already -eq 0 ]]; then
        resolved_configs+=("$candidate")
      fi
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
    stripped_file="$(mktemp)"
    if ! strip_jsonc_comments "$cfg" > "$stripped_file"; then
      echo "::error::Config file '$cfg' could not be read/searched; failing closed (exit 2)." >&2
      rm -f "$stripped_file"
      exit 2
    fi
    filtered_file="$(mktemp)"
    if ! strip_disabled_providers "$stripped_file" > "$filtered_file"; then
      echo "::error::Config file '$cfg' could not be read/searched; failing closed (exit 2)." >&2
      rm -f "$stripped_file" "$filtered_file"
      exit 2
    fi
    rm -f "$stripped_file"
    grep_status=0
    matches="$(grep -nF -e "\"$provider\"" -e "\"$provider/" "$filtered_file")" || grep_status=$?
    rm -f "$filtered_file"
    if [[ $grep_status -eq 2 ]]; then
      echo "::error::Config file '$cfg' could not be searched (grep error); failing closed (exit 2)." >&2
      exit 2
    fi
    if [[ $grep_status -ne 0 && $grep_status -ne 1 ]]; then
      echo "::error::Searching config file '$cfg' failed with unexpected status $grep_status; failing closed (exit 2)." >&2
      exit 2
    fi
    if [[ $grep_status -eq 0 ]]; then
      count="$(printf '%s\n' "$matches" | wc -l)"
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
