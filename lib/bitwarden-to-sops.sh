#!/usr/bin/env bash
# Description: Fetch a secret from Bitwarden (Vault or Secrets Manager) and push/encrypt
# it into a SOPS/age encrypted YAML file in the repository.
#
# Usage:
#   Interactive:
#     ./lib/bitwarden-to-sops.sh
#
#   CLI:
#     ./lib/bitwarden-to-sops.sh --item "GitHub" --password --target secrets/sops/common/github.yaml --key github_token
#     ./lib/bitwarden-to-sops.sh --item "Cloudflare" --field API_TOKEN --target secrets/sops/common/cloudflare.yaml
#     ./lib/bitwarden-to-sops.sh --item "MyService" --all-fields --target secrets/sops/common/myservice.yaml
#     ./lib/bitwarden-to-sops.sh --bws-secret "PROD_DB_URL" --target secrets/sops/common/db.yaml

set -euo pipefail

# Locate repository root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel 2>/dev/null || dirname "$SCRIPT_DIR")"

# Ensure PATH includes ~/.local/bin
if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]] && [ -d "$HOME/.local/bin" ]; then
  export PATH="$HOME/.local/bin:$PATH"
fi

if ! command -v sops >/dev/null 2>&1; then
  echo "Error: sops CLI is not found in PATH." >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "Error: jq CLI is not found in PATH." >&2
  exit 1
fi

ITEM_QUERY=""
SPECIFIC_FIELD=""
ALL_FIELDS=false
USE_BWS=false
BWS_SECRET_QUERY=""
TARGET_FILE=""
TARGET_KEY=""
KEY_PREFIX=""
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
      ALL_FIELDS=true
      shift 1
      ;;
    --bws-secret)
      USE_BWS=true
      BWS_SECRET_QUERY="$2"
      shift 2
      ;;
    --target)
      TARGET_FILE="$2"
      shift 2
      ;;
    --key)
      TARGET_KEY="$2"
      shift 2
      ;;
    --prefix)
      KEY_PREFIX="$2"
      shift 2
      ;;
    --no-auto-env)
      AUTO_ENV=false
      shift 1
      ;;
    -h|--help)
      cat << 'USAGE'
Usage: ./lib/bitwarden-to-sops.sh [options]

Source Options:
  --item <name_or_id>        Bitwarden Vault item name or ID (bw)
  --field <field_name>       Specific custom field, or 'password', 'username', 'notes'
  --password                 Shortcut for --field password
  --username                 Shortcut for --field username
  --notes                    Shortcut for --field notes
  --all-fields               Push all custom fields from the item into the target SOPS file
  --bws-secret <id_or_key>   Bitwarden Secrets Manager secret (bws)

Destination Options:
  --target <path>            Target SOPS YAML file (e.g. secrets/sops/common/app.yaml)
  --key <key_name>           Key name in the SOPS file (defaults to field name)
  --prefix <prefix>          Prefix key names (when using --all-fields)

General Options:
  --no-auto-env              Do not auto-run lib/bitwarden-env.sh if session is locked
  -h, --help                 Show this help message

If run without arguments, enters interactive mode.
USAGE
      exit 0
      ;;
    *)
      echo "Error: Unknown option $1" >&2
      exit 1
      ;;
  esac
done

# Auto-environment check
if [ "$AUTO_ENV" = true ] && [ -f "${REPO_ROOT}/lib/bitwarden-env.sh" ]; then
  if [ "$USE_BWS" = true ] && [ -z "${BWS_ACCESS_TOKEN:-}" ]; then
    source "${REPO_ROOT}/lib/bitwarden-env.sh" >/dev/null 2>&1 || true
  elif [ "$USE_BWS" = false ] && [ -z "${BW_SESSION:-}" ]; then
    if command -v bw >/dev/null 2>&1; then
      BW_STATUS="$(bw status 2>/dev/null | jq -r '.status // "unknown"')"
      if [ "$BW_STATUS" != "unlocked" ]; then
        source "${REPO_ROOT}/lib/bitwarden-env.sh" >/dev/null 2>&1 || true
      fi
    fi
  fi
fi

# Sanitize YAML key name (lowercase, underscores)
sanitize_key() {
  local k="$1"
  local clean
  clean="$(echo "$k" | tr '[:upper:]' '[:lower:]' | sed -E 's/[^a-zA-Z0-9_]/_/g')"
  if [[ "$clean" =~ ^[0-9] ]]; then
    clean="_$clean"
  fi
  echo "$clean"
}

# Resolve target path relative to REPO_ROOT if relative
resolve_target_path() {
  local t="$1"
  if [[ "$t" != /* ]]; then
    t="${REPO_ROOT}/${t}"
  fi
  echo "$t"
}

# -------------------------------------------------------------
# INTERACTIVE MODE
# -------------------------------------------------------------
if [ -z "$ITEM_QUERY" ] && [ -z "$BWS_SECRET_QUERY" ]; then
  echo "=== Push Bitwarden Secret to SOPS/Age ==="
  echo ""
  echo "Select source:"
  echo "  1) Bitwarden Vault (bw)"
  echo "  2) Bitwarden Secrets Manager (bws)"
  read -r -p "Selection [1]: " SOURCE_CHOICE
  SOURCE_CHOICE="${SOURCE_CHOICE:-1}"

  if [ "$SOURCE_CHOICE" = "2" ]; then
    USE_BWS=true
    if [ -z "${BWS_ACCESS_TOKEN:-}" ]; then
      read -r -s -p "Enter BWS_ACCESS_TOKEN: " BWS_ACCESS_TOKEN
      echo ""
      export BWS_ACCESS_TOKEN
    fi
    echo "Fetching secrets from Secrets Manager..."
    SECRETS_LIST="$(bws secret list 2>/dev/null)" || {
      echo "Error: Failed to list secrets from bws." >&2
      exit 1
    }
    COUNT="$(echo "$SECRETS_LIST" | jq 'length')"
    if [ "$COUNT" -eq 0 ]; then
      echo "No secrets found in Secrets Manager." >&2
      exit 1
    fi
    echo "Available Secrets:"
    echo "$SECRETS_LIST" | jq -r 'to_entries[] | "\(.key + 1)) \(.value.key) (ID: \(.value.id))"'
    read -r -p "Select secret number or enter ID/Key: " SEC_SELECTION
    if [[ "$SEC_SELECTION" =~ ^[0-9]+$ ]] && [ "$SEC_SELECTION" -le "$COUNT" ]; then
      INDEX=$((SEC_SELECTION - 1))
      BWS_SECRET_QUERY="$(echo "$SECRETS_LIST" | jq -r ".[$INDEX].id")"
    else
      BWS_SECRET_QUERY="$SEC_SELECTION"
    fi

  else
    # Vault mode (bw)
    if [ -z "${BW_SESSION:-}" ]; then
      BW_STATUS="$(bw status 2>/dev/null | jq -r '.status // "unknown"')"
      if [ "$BW_STATUS" != "unlocked" ]; then
        echo "Bitwarden Vault is locked. Attempting unlock..."
        BW_RAW="$(bw unlock --raw)" || {
          echo "Unlock failed. Please unlock your vault and retry." >&2
          exit 1
        }
        export BW_SESSION="$BW_RAW"
      fi
    fi

    read -r -p "Search Bitwarden Vault for item: " SEARCH_TERM
    MATCHES="$(bw list items --search "$SEARCH_TERM" 2>/dev/null)" || {
      echo "Error searching vault." >&2
      exit 1
    }
    MATCH_COUNT="$(echo "$MATCHES" | jq 'length')"
    if [ "$MATCH_COUNT" -eq 0 ]; then
      echo "No items found matching '$SEARCH_TERM'." >&2
      exit 1
    fi

    echo "Matching Items:"
    echo "$MATCHES" | jq -r 'to_entries[] | "\(.key + 1)) \(.value.name) (ID: \(.value.id))"'
    read -r -p "Select item number [1]: " ITEM_SEL
    ITEM_SEL="${ITEM_SEL:-1}"
    INDEX=$((ITEM_SEL - 1))
    ITEM_QUERY="$(echo "$MATCHES" | jq -r ".[$INDEX].id")"
    SELECTED_NAME="$(echo "$MATCHES" | jq -r ".[$INDEX].name")"

    # Field selection
    echo ""
    echo "Select field to push for '$SELECTED_NAME':"
    echo "  1) Password"
    echo "  2) Username"
    echo "  3) Notes"
    echo "  4) All custom fields"
    echo "  5) Specific custom field"
    read -r -p "Selection [1]: " FIELD_CHOICE
    FIELD_CHOICE="${FIELD_CHOICE:-1}"

    case "$FIELD_CHOICE" in
      1) SPECIFIC_FIELD="password" ;;
      2) SPECIFIC_FIELD="username" ;;
      3) SPECIFIC_FIELD="notes" ;;
      4) ALL_FIELDS=true ;;
      5)
        read -r -p "Enter custom field name: " SPECIFIC_FIELD
        ;;
    esac
  fi

  # Prompt for target file
  echo ""
  DEFAULT_TARGET="secrets/sops/common/$(sanitize_key "${SELECTED_NAME:-bitwarden}").yaml"
  read -r -p "Target SOPS file [$DEFAULT_TARGET]: " INPUT_TARGET
  TARGET_FILE="${INPUT_TARGET:-$DEFAULT_TARGET}"

  # Prompt for key if single field
  if [ "$ALL_FIELDS" = false ] && [ -z "$TARGET_KEY" ]; then
    DEFAULT_KEY="$(sanitize_key "${SPECIFIC_FIELD:-secret}")"
    read -r -p "Target SOPS key name [$DEFAULT_KEY]: " INPUT_KEY
    TARGET_KEY="${INPUT_KEY:-$DEFAULT_KEY}"
  fi
fi

# Ensure target file path is resolved
TARGET_FILE="$(resolve_target_path "$TARGET_FILE")"

# -------------------------------------------------------------
# FETCH SECRET(S) TO PUSH
# Format: KEY<TAB>VALUE
# -------------------------------------------------------------
PAIRS=""

add_entry() {
  local k="$1"
  local v="$2"
  PAIRS+="${k}"$'\t'"${v}"$'\n'
}

if [ "$USE_BWS" = true ]; then
  if ! command -v bws >/dev/null 2>&1; then
    echo "Error: bws CLI not found." >&2
    exit 1
  fi
  RAW_JSON="$(bws secret get "$BWS_SECRET_QUERY" 2>/dev/null)" || {
    echo "Error: Failed to fetch secret '$BWS_SECRET_QUERY' from bws." >&2
    exit 1
  }
  SEC_KEY="$(echo "$RAW_JSON" | jq -r '.key')"
  SEC_VAL="$(echo "$RAW_JSON" | jq -r '.value')"
  DEST_KEY="${TARGET_KEY:-$(sanitize_key "$SEC_KEY")}"
  add_entry "$DEST_KEY" "$SEC_VAL"

else
  if ! command -v bw >/dev/null 2>&1; then
    echo "Error: bw CLI not found." >&2
    exit 1
  fi
  ITEM_JSON="$(bw get item "$ITEM_QUERY" 2>/dev/null)" || {
    echo "Error: Failed to get item '$ITEM_QUERY' from Bitwarden. Check if vault is unlocked." >&2
    exit 1
  }
  ITEM_NAME="$(echo "$ITEM_JSON" | jq -r '.name')"

  if [ "$ALL_FIELDS" = true ]; then
    while IFS=$'\t' read -r k v; do
      if [ -n "$k" ]; then
        DEST_K="$(sanitize_key "${KEY_PREFIX}${k}")"
        add_entry "$DEST_K" "$v"
      fi
    done < <(echo "$ITEM_JSON" | jq -r '(.fields // [])[] | "\(.name)\t\(.value)"')

    if [ -z "$PAIRS" ]; then
      echo "Warning: Item '$ITEM_NAME' has no custom fields to push." >&2
      exit 1
    fi
  else
    case "${SPECIFIC_FIELD:-password}" in
      password)
        VAL="$(echo "$ITEM_JSON" | jq -r '.login.password // empty')"
        DEST_K="${TARGET_KEY:-$(sanitize_key "${ITEM_NAME}_password")}"
        ;;
      username)
        VAL="$(echo "$ITEM_JSON" | jq -r '.login.username // empty')"
        DEST_K="${TARGET_KEY:-$(sanitize_key "${ITEM_NAME}_username")}"
        ;;
      notes)
        VAL="$(echo "$ITEM_JSON" | jq -r '.notes // empty')"
        DEST_K="${TARGET_KEY:-$(sanitize_key "${ITEM_NAME}_notes")}"
        ;;
      *)
        VAL="$(echo "$ITEM_JSON" | jq -r --arg f "$SPECIFIC_FIELD" '(.fields // [])[] | select(.name == $f) | .value // empty')"
        if [ -z "$VAL" ]; then
          echo "Error: Field '$SPECIFIC_FIELD' not found on item '$ITEM_NAME'." >&2
          exit 1
        fi
        DEST_K="${TARGET_KEY:-$(sanitize_key "$SPECIFIC_FIELD")}"
        ;;
    esac
    [ -n "$VAL" ] && add_entry "$DEST_K" "$VAL"
  fi
fi

if [ -z "$PAIRS" ]; then
  echo "Error: No secret content extracted." >&2
  exit 1
fi

# -------------------------------------------------------------
# WRITE / ENCRYPT INTO SOPS
# -------------------------------------------------------------
mkdir -p "$(dirname "$TARGET_FILE")"

# If file does not exist, initialize it with sops -e
if [ ! -f "$TARGET_FILE" ]; then
  echo "Target file $TARGET_FILE does not exist. Initializing new SOPS/age encrypted file..."
  FIRST_LINE=true
  TMP_YAML=""
  while IFS=$'\t' read -r k v; do
    if [ -n "$k" ]; then
      json_v="$(jq -nc --arg v "$v" '$v')"
      TMP_YAML+="${k}: ${json_v}"$'\n'
    fi
  done <<< "$PAIRS"

  sops -e --filename-override "$TARGET_FILE" <(echo -n "$TMP_YAML") > "$TARGET_FILE"
  echo "Created and encrypted $TARGET_FILE successfully."

else
  # File exists: update each key using sops set
  while IFS=$'\t' read -r k v; do
    if [ -n "$k" ]; then
      json_v="$(jq -nc --arg v "$v" '$v')"
      sops set "$TARGET_FILE" "[\"$k\"]" "$json_v"
      echo "  [✓] Updated key '$k' (${#v} chars) in $TARGET_FILE"
    fi
  done <<< "$PAIRS"
fi

echo "Done. Secret(s) pushed to SOPS: $TARGET_FILE"
