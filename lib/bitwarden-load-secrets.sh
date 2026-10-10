#!/usr/bin/env bash
# Description: Fetch secrets from Bitwarden (Vault or Secrets Manager) and
# load them into environment variables, .env format, or JSON for ingestion.
#
# Usage:
#   source lib/bitwarden-load-secrets.sh --item "MyService"
#   ./lib/bitwarden-load-secrets.sh --item "MyService" --dotenv > .env
#   ./lib/bitwarden-load-secrets.sh --bws --format json

is_sourced() {
  [[ "${BASH_SOURCE[0]}" != "${0}" ]]
}

die() {
  echo "Error: $*" >&2
  if is_sourced; then return 1; else exit 1; fi
}

# Locate repository root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel 2>/dev/null || dirname "$SCRIPT_DIR")"

# Ensure PATH includes ~/.local/bin
if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]] && [ -d "$HOME/.local/bin" ]; then
  export PATH="$HOME/.local/bin:$PATH"
fi

ITEM_QUERY=""
SPECIFIC_FIELD=""
USE_BWS=false
BWS_SECRET_QUERY=""
BWS_PROJECT_QUERY=""
INCLUDE_LOGIN=false
INCLUDE_NOTES=false
VAR_PREFIX=""
FORMAT=""
AUTO_ENV=true

# Parse arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    --item)
      ITEM_QUERY="$2"
      shift 2
      ;;
    --field)
      SPECIFIC_FIELD="$2"
      shift 2
      ;;
    --password)
      SPECIFIC_FIELD="password"
      shift 1
      ;;
    --username)
      SPECIFIC_FIELD="username"
      shift 1
      ;;
    --notes)
      SPECIFIC_FIELD="notes"
      shift 1
      ;;
    --all-fields)
      SPECIFIC_FIELD=""
      shift 1
      ;;
    --include-login)
      INCLUDE_LOGIN=true
      shift 1
      ;;
    --include-notes)
      INCLUDE_NOTES=true
      shift 1
      ;;
    --bws)
      USE_BWS=true
      shift 1
      ;;
    --secret)
      USE_BWS=true
      BWS_SECRET_QUERY="$2"
      shift 2
      ;;
    --project)
      USE_BWS=true
      BWS_PROJECT_QUERY="$2"
      shift 2
      ;;
    --prefix)
      VAR_PREFIX="$2"
      shift 2
      ;;
    --format)
      FORMAT="$2"
      shift 2
      ;;
    --dotenv|--env)
      FORMAT="env"
      shift 1
      ;;
    --json)
      FORMAT="json"
      shift 1
      ;;
    --no-auto-env)
      AUTO_ENV=false
      shift 1
      ;;
    -h|--help)
      cat << 'USAGE'
Usage:
  source lib/bitwarden-load-secrets.sh [options]
  ./lib/bitwarden-load-secrets.sh [options]

Vault Item Mode (bw):
  --item <name_or_id>        Fetch item from Bitwarden Vault
  --field <field_name>       Extract specific custom field, or 'password', 'username', 'notes'
  --password                 Shortcut for --field password
  --username                 Shortcut for --field username
  --notes                    Shortcut for --field notes
  --all-fields               Extract all custom fields (default)
  --include-login            Include USERNAME and PASSWORD alongside custom fields
  --include-notes            Include NOTES alongside custom fields

Secrets Manager Mode (bws):
  --bws                      Fetch from Bitwarden Secrets Manager
  --secret <id_or_key>       Fetch a specific secret
  --project <project_id>     Fetch all secrets in a specific project

Transform & Output:
  --prefix <PREFIX>          Prefix variable names (e.g. --prefix APP_)
  --format <export|env|json> Output format when executed directly (default: export)
  --dotenv / --env           Shortcut for --format env
  --json                     Shortcut for --format json
  --no-auto-env              Do not auto-run lib/bitwarden-env.sh if locked/missing
  -h, --help                 Show this help message
USAGE
      if is_sourced; then return 0; else exit 0; fi
      ;;
    *)
      die "Unknown option: $1"
      ;;
  esac
done

# Default format
if [ -z "$FORMAT" ]; then
  if is_sourced; then
    FORMAT="source"
  else
    FORMAT="export"
  fi
fi

# Auto-environment check
if [ "$AUTO_ENV" = true ] && [ -f "${REPO_ROOT}/lib/bitwarden-env.sh" ]; then
  if [ "$USE_BWS" = true ] && [ -z "${BWS_ACCESS_TOKEN:-}" ]; then
    # Attempt to load bws token quietly
    source "${REPO_ROOT}/lib/bitwarden-env.sh" >/dev/null 2>&1 || true
  elif [ "$USE_BWS" = false ] && [ -z "${BW_SESSION:-}" ]; then
    # Check if bw is locked or unauthenticated
    if command -v bw >/dev/null 2>&1; then
      BW_STATUS="$(bw status 2>/dev/null | jq -r '.status // "unknown"')"
      if [ "$BW_STATUS" != "unlocked" ]; then
        source "${REPO_ROOT}/lib/bitwarden-env.sh" >/dev/null 2>&1 || true
      fi
    fi
  fi
fi

if ! command -v jq >/dev/null 2>&1; then
  die "jq CLI is required but not found in PATH."
fi

# Sanitize variable names into valid bash identifiers
sanitize_var_name() {
  local name="$1"
  local clean
  clean="$(echo "$name" | sed -E 's/[^a-zA-Z0-9_]/_/g')"
  if [[ "$clean" =~ ^[0-9] ]]; then
    clean="_$clean"
  fi
  echo "$clean"
}

# Temporary associative-like storage using newline-delimited pairs
# Format: KEY<TAB>VALUE
PAIRS=""

add_pair() {
  local k="$1"
  local v="$2"
  local safe_k
  safe_k="$(sanitize_var_name "${VAR_PREFIX}${k}")"
  if [ -n "$safe_k" ]; then
    PAIRS+="${safe_k}"$'\t'"${v}"$'\n'
  fi
}

# -------------------------------------------------------------
# MODE 1: Bitwarden Secrets Manager (bws)
# -------------------------------------------------------------
if [ "$USE_BWS" = true ]; then
  if ! command -v bws >/dev/null 2>&1; then
    die "bws CLI not found. Run lib/bitwarden-env.sh or check ~/.local/bin."
  fi
  if [ -z "${BWS_ACCESS_TOKEN:-}" ]; then
    die "BWS_ACCESS_TOKEN is not set. Run: source lib/bitwarden-env.sh or set BWS_ACCESS_TOKEN."
  fi

  if [ -n "$BWS_SECRET_QUERY" ]; then
    # Single secret
    RAW_JSON="$(bws secret get "$BWS_SECRET_QUERY" 2>/dev/null)" || die "Failed to get secret '$BWS_SECRET_QUERY' from bws."
    KEY="$(echo "$RAW_JSON" | jq -r '.key // empty')"
    VAL="$(echo "$RAW_JSON" | jq -r '.value // empty')"
    [ -n "$KEY" ] && add_pair "$KEY" "$VAL"
  else
    # List of secrets (optional project filter)
    ARGS=()
    if [ -n "$BWS_PROJECT_QUERY" ]; then
      ARGS+=("$BWS_PROJECT_QUERY")
    fi
    RAW_JSON="$(bws secret list "${ARGS[@]}" 2>/dev/null)" || die "Failed to list secrets from bws."
    while IFS=$'\t' read -r k v; do
      [ -n "$k" ] && add_pair "$k" "$v"
    done < <(echo "$RAW_JSON" | jq -r '.[] | "\(.key)\t\(.value)"')
  fi

# -------------------------------------------------------------
# MODE 2: Bitwarden Vault (bw)
# -------------------------------------------------------------
else
  if [ -z "$ITEM_QUERY" ]; then
    die "Missing item name or ID. Specify --item <name_or_id> or use --bws for Secrets Manager."
  fi
  if ! command -v bw >/dev/null 2>&1; then
    die "bw CLI not found. Run lib/bitwarden-env.sh or check ~/.local/bin."
  fi

  ITEM_JSON="$(bw get item "$ITEM_QUERY" 2>/dev/null)" || die "Failed to get item '$ITEM_QUERY' from Bitwarden. Check if vault is unlocked (bw status)."

  ITEM_NAME="$(echo "$ITEM_JSON" | jq -r '.name // "ITEM"')"

  # Handle single specific field
  if [ -n "$SPECIFIC_FIELD" ]; then
    case "$SPECIFIC_FIELD" in
      password)
        VAL="$(echo "$ITEM_JSON" | jq -r '.login.password // empty')"
        add_pair "${SPECIFIC_FIELD^^}" "$VAL"
        ;;
      username)
        VAL="$(echo "$ITEM_JSON" | jq -r '.login.username // empty')"
        add_pair "${SPECIFIC_FIELD^^}" "$VAL"
        ;;
      notes)
        VAL="$(echo "$ITEM_JSON" | jq -r '.notes // empty')"
        add_pair "${SPECIFIC_FIELD^^}" "$VAL"
        ;;
      *)
        # Custom field match
        VAL="$(echo "$ITEM_JSON" | jq -r --arg f "$SPECIFIC_FIELD" '(.fields // [])[] | select(.name == $f) | .value // empty')"
        if [ -z "$VAL" ]; then
          die "Field '$SPECIFIC_FIELD' not found on item '$ITEM_NAME'."
        fi
        add_pair "$SPECIFIC_FIELD" "$VAL"
        ;;
    esac

  # Extract all custom fields and optional login/notes
  else
    # 1. Custom fields
    while IFS=$'\t' read -r k v; do
      [ -n "$k" ] && add_pair "$k" "$v"
    done < <(echo "$ITEM_JSON" | jq -r '(.fields // [])[] | "\(.name)\t\(.value)"')

    # 2. Login fields if requested or if no custom fields exist
    NUM_CUSTOM="$(echo "$ITEM_JSON" | jq '(.fields // []) | length')"
    if [ "$INCLUDE_LOGIN" = true ] || [ "$NUM_CUSTOM" -eq 0 ]; then
      USER_VAL="$(echo "$ITEM_JSON" | jq -r '.login.username // empty')"
      PASS_VAL="$(echo "$ITEM_JSON" | jq -r '.login.password // empty')"
      [ -n "$USER_VAL" ] && add_pair "USERNAME" "$USER_VAL"
      [ -n "$PASS_VAL" ] && add_pair "PASSWORD" "$PASS_VAL"
    fi

    # 3. Notes if requested
    if [ "$INCLUDE_NOTES" = true ]; then
      NOTES_VAL="$(echo "$ITEM_JSON" | jq -r '.notes // empty')"
      [ -n "$NOTES_VAL" ] && add_pair "NOTES" "$NOTES_VAL"
    fi
  fi
fi

if [ -z "$PAIRS" ]; then
  die "No secret fields found to load."
fi

# -------------------------------------------------------------
# Output / Export Dispatcher
# -------------------------------------------------------------
COUNT=0
LOADED_NAMES=()

case "$FORMAT" in
  source)
    while IFS=$'\t' read -r k v; do
      if [ -n "$k" ]; then
        export "$k"="$v"
        LOADED_NAMES+=("$k")
        COUNT=$((COUNT + 1))
      fi
    done <<< "$PAIRS"
    echo "[bw-load] Exported $COUNT variable(s) into current shell: ${LOADED_NAMES[*]}"
    ;;

  export)
    while IFS=$'\t' read -r k v; do
      if [ -n "$k" ]; then
        escaped_val="$(printf '%s' "$v" | jq -Rr @sh)"
        printf 'export %s=%s\n' "$k" "$escaped_val"
      fi
    done <<< "$PAIRS"
    ;;

  env)
    while IFS=$'\t' read -r k v; do
      if [ -n "$k" ]; then
        escaped_val="$(printf '%s' "$v" | jq -Rr @sh)"
        printf '%s=%s\n' "$k" "$escaped_val"
      fi
    done <<< "$PAIRS"
    ;;

  json)
    # Build JSON object from pairs
    jq -n -R '
      [inputs | select(length > 0) | split("\t")] |
      map({(.[0]): .[1]}) | add // {}
    ' <<< "$PAIRS"
    ;;

  *)
    die "Unknown format '$FORMAT'. Supported: export, env, json."
    ;;
esac
