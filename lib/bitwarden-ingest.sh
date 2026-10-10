#!/usr/bin/env bash
set -euo pipefail

# Find repository root
REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || dirname "$(dirname "$(readlink -f "$0")")")"
SECRET_FILE="${REPO_ROOT}/secrets/sops/common/bitwarden.yaml"

if [ ! -f "$SECRET_FILE" ]; then
  echo "Error: Secret file not found at $SECRET_FILE" >&2
  exit 1
fi

if ! command -v sops >/dev/null 2>&1; then
  echo "Error: sops is not installed or not in PATH." >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "Error: jq is not installed or not in PATH." >&2
  exit 1
fi

# Decrypt existing secrets if possible to allow selective updates
EXISTING_JSON="{}"
if sops -d --output-type json "$SECRET_FILE" >/dev/null 2>&1; then
  EXISTING_JSON="$(sops -d --output-type json "$SECRET_FILE")"
fi

EXISTING_BWS_TOKEN="$(echo "$EXISTING_JSON" | jq -r '.bws_access_token // empty')"
EXISTING_BW_ID="$(echo "$EXISTING_JSON" | jq -r '.bw_client_id // empty')"
EXISTING_BW_SECRET="$(echo "$EXISTING_JSON" | jq -r '.bw_client_secret // empty')"
EXISTING_BW_PASS="$(echo "$EXISTING_JSON" | jq -r '.bw_password // empty')"

NEW_BWS_TOKEN=""
NEW_BW_ID=""
NEW_BW_SECRET=""
NEW_BW_PASS=""

DO_UPDATE_BWS=false
DO_UPDATE_ID=false
DO_UPDATE_SECRET=false
DO_UPDATE_PASS=false
INTERACTIVE=true

# Parse arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    --bws-token)
      NEW_BWS_TOKEN="$2"
      DO_UPDATE_BWS=true
      INTERACTIVE=false
      shift 2
      ;;
    --client-id)
      NEW_BW_ID="$2"
      DO_UPDATE_ID=true
      INTERACTIVE=false
      shift 2
      ;;
    --client-secret)
      NEW_BW_SECRET="$2"
      DO_UPDATE_SECRET=true
      INTERACTIVE=false
      shift 2
      ;;
    --password)
      NEW_BW_PASS="$2"
      DO_UPDATE_PASS=true
      INTERACTIVE=false
      shift 2
      ;;
    -h|--help)
      echo "Usage: $0 [options]"
      echo ""
      echo "Options:"
      echo "  --bws-token <token>       Bitwarden Secrets Manager access token (BWS_ACCESS_TOKEN)"
      echo "  --client-id <id>          Bitwarden API Client ID (BW_CLIENTID)"
      echo "  --client-secret <secret>  Bitwarden API Client Secret (BW_CLIENTSECRET)"
      echo "  --password <password>     Bitwarden Master Password (optional)"
      echo "  -h, --help                Show this help message"
      echo ""
      echo "If run without arguments, runs in interactive mode."
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
  esac
done

if [ "$INTERACTIVE" = true ]; then
  echo "=== Bitwarden Secret Ingestion ==="
  echo "Target: $SECRET_FILE (encrypted with SOPS + age)"
  echo "Press Enter without typing to keep existing values."
  echo ""

  # 1. BWS Access Token
  PROMPT="Enter BWS_ACCESS_TOKEN"
  if [ -n "$EXISTING_BWS_TOKEN" ]; then
    PROMPT="$PROMPT [currently set: ${#EXISTING_BWS_TOKEN} chars]"
  fi
  read -r -s -p "$PROMPT: " INPUT_BWS_TOKEN
  echo ""
  if [ -n "$INPUT_BWS_TOKEN" ]; then
    NEW_BWS_TOKEN="$INPUT_BWS_TOKEN"
    DO_UPDATE_BWS=true
  fi

  # 2. BW Client ID
  PROMPT="Enter BW_CLIENTID"
  if [ -n "$EXISTING_BW_ID" ]; then
    PROMPT="$PROMPT [currently set: ${EXISTING_BW_ID:0:8}...]"
  fi
  read -r -p "$PROMPT: " INPUT_BW_ID
  if [ -n "$INPUT_BW_ID" ]; then
    NEW_BW_ID="$INPUT_BW_ID"
    DO_UPDATE_ID=true
  fi

  # 3. BW Client Secret
  PROMPT="Enter BW_CLIENTSECRET"
  if [ -n "$EXISTING_BW_SECRET" ]; then
    PROMPT="$PROMPT [currently set: ${#EXISTING_BW_SECRET} chars]"
  fi
  read -r -s -p "$PROMPT: " INPUT_BW_SECRET
  echo ""
  if [ -n "$INPUT_BW_SECRET" ]; then
    NEW_BW_SECRET="$INPUT_BW_SECRET"
    DO_UPDATE_SECRET=true
  fi

  # 4. BW Master Password
  PROMPT="Enter Bitwarden Master Password (optional, for auto-unlock)"
  if [ -n "$EXISTING_BW_PASS" ]; then
    PROMPT="$PROMPT [currently set]"
  fi
  read -r -s -p "$PROMPT: " INPUT_BW_PASS
  echo ""
  if [ -n "$INPUT_BW_PASS" ]; then
    NEW_BW_PASS="$INPUT_BW_PASS"
    DO_UPDATE_PASS=true
  fi
fi

UPDATED=0
set_key() {
  local key="$1"
  local val="$2"
  local json_val
  json_val="$(jq -nc --arg v "$val" '$v')"
  sops set "$SECRET_FILE" "[\"$key\"]" "$json_val"
  UPDATED=$((UPDATED + 1))
  if [ -z "$val" ]; then
    echo "  [✓] Cleared $key"
  else
    echo "  [✓] Updated $key (${#val} chars)"
  fi
}

echo "Updating encrypted secrets in $SECRET_FILE..."
if [ "$DO_UPDATE_BWS" = true ]; then set_key "bws_access_token" "$NEW_BWS_TOKEN"; fi
if [ "$DO_UPDATE_ID" = true ]; then set_key "bw_client_id" "$NEW_BW_ID"; fi
if [ "$DO_UPDATE_SECRET" = true ]; then set_key "bw_client_secret" "$NEW_BW_SECRET"; fi
if [ "$DO_UPDATE_PASS" = true ]; then set_key "bw_password" "$NEW_BW_PASS"; fi

if [ "$UPDATED" -eq 0 ]; then
  echo "No changes made."
else
  echo "Done. $UPDATED secret(s) encrypted and saved."
fi
