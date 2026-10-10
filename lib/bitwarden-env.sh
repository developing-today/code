#!/usr/bin/env bash
# Description: Decrypts Bitwarden credentials and exports environment variables
# for bws (Secrets Manager) and bw (Vault CLI).
#
# Usage:
#   source lib/bitwarden-env.sh
#   or
#   . lib/bitwarden-env.sh

is_sourced() {
  [[ "${BASH_SOURCE[0]}" != "${0}" ]]
}

if ! is_sourced; then
  echo "Notice: You ran this script directly instead of sourcing it." >&2
  echo "To export variables into your active shell, run:" >&2
  echo "  source ${BASH_SOURCE[0]}" >&2
  echo ""
fi

# Ensure ~/.local/bin is in PATH if binaries reside there
if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]] && [ -d "$HOME/.local/bin" ]; then
  export PATH="$HOME/.local/bin:$PATH"
fi

# Locate repository root and secret file
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel 2>/dev/null || dirname "$SCRIPT_DIR")"
SECRET_FILE="${REPO_ROOT}/secrets/sops/common/bitwarden.yaml"

if [ ! -f "$SECRET_FILE" ]; then
  echo "Warning: Secret file $SECRET_FILE not found." >&2
  echo "Run ./lib/bitwarden-ingest.sh to initialize secrets." >&2
  if is_sourced; then return 1; else exit 1; fi
fi

if ! command -v sops >/dev/null 2>&1; then
  echo "Error: sops CLI not found in PATH." >&2
  if is_sourced; then return 1; else exit 1; fi
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "Error: jq CLI not found in PATH." >&2
  if is_sourced; then return 1; else exit 1; fi
fi

# Decrypt in-memory
DEC_JSON="$(sops -d --output-type json "$SECRET_FILE" 2>/dev/null)" || {
  echo "Error: Failed to decrypt $SECRET_FILE using sops." >&2
  if is_sourced; then return 1; else exit 1; fi
}

_BWS_TOKEN="$(echo "$DEC_JSON" | jq -r '.bws_access_token // empty')"
_BW_ID="$(echo "$DEC_JSON" | jq -r '.bw_client_id // empty')"
_BW_SECRET="$(echo "$DEC_JSON" | jq -r '.bw_client_secret // empty')"
_BW_PASS="$(echo "$DEC_JSON" | jq -r '.bw_password // empty')"

# 1. Bitwarden Secrets Manager CLI (bws)
if [ -n "$_BWS_TOKEN" ]; then
  export BWS_ACCESS_TOKEN="$_BWS_TOKEN"
  echo "[bws] BWS_ACCESS_TOKEN exported successfully."
else
  echo "[bws] bws_access_token is empty in $SECRET_FILE."
fi

# 2. Bitwarden Vault CLI (bw)
if [ -n "$_BW_ID" ] && [ -n "$_BW_SECRET" ]; then
  export BW_CLIENTID="$_BW_ID"
  export BW_CLIENTSECRET="$_BW_SECRET"
fi

if command -v bw >/dev/null 2>&1; then
  BW_STATUS="$(bw status 2>/dev/null | jq -r '.status // "unknown"')"

  if [ "$BW_STATUS" = "unauthenticated" ] && [ -n "${BW_CLIENTID:-}" ] && [ -n "${BW_CLIENTSECRET:-}" ]; then
    echo "[bw] Logging into Bitwarden Vault via API key..."
    bw login --apikey >/dev/null 2>&1 || true
    BW_STATUS="$(bw status 2>/dev/null | jq -r '.status // "unknown"')"
  fi

  if [ "$BW_STATUS" = "locked" ]; then
    if [ -n "$_BW_PASS" ]; then
      echo "[bw] Unlocking Bitwarden Vault..."
      export BW_PASSWORD="$_BW_PASS"
      _RAW_SESSION="$(bw unlock --passwordenv BW_PASSWORD --raw 2>/dev/null)" || true
      unset BW_PASSWORD
      if [ -n "$_RAW_SESSION" ]; then
        export BW_SESSION="$_RAW_SESSION"
        echo "[bw] BW_SESSION exported successfully."
      else
        echo "[bw] Unlock failed. Please check master password." >&2
      fi
      unset _RAW_SESSION
    else
      echo "[bw] Vault is locked. Run 'export BW_SESSION=\$(bw unlock --raw)' to unlock."
    fi
  elif [ "$BW_STATUS" = "unlocked" ]; then
    echo "[bw] Vault is already unlocked and active."
  elif [ "$BW_STATUS" = "unauthenticated" ]; then
    echo "[bw] bw_client_id/bw_client_secret empty or not yet authenticated."
  fi
fi

unset DEC_JSON _BWS_TOKEN _BW_ID _BW_SECRET _BW_PASS BW_STATUS
