#!/usr/bin/env bash
# agent-auth -- check and bootstrap the auth that every hidden shell inherits.
#
#   agent-auth status   OK/FAIL for bws, 1Password (op) and Jules, from a hidden-shell view
#   agent-auth login    interactively fix whatever `status` reports as missing
#   agent-auth env      which auth variables the hook exports (names only)
#
# See pkgs/agent-shell-env/env.sh for the hook itself.
set -uo pipefail
# coreutils/grep pinned; `op` (setgid wrapper in /run/wrappers/bin), `bws` and
# `jules` deliberately come from the system PATH.
export PATH="@toolsPath@:/run/wrappers/bin:${PATH:-/run/current-system/sw/bin}"

HOOK="@out@/share/agent-shell-env/env.sh"
CFG="${XDG_CONFIG_HOME:-$HOME/.config}"
OP_TOKEN_FILE="$CFG/op/service-account-token"

# shellcheck source=/dev/null
. "$HOOK"

ok() { printf '  \033[32mOK\033[0m    %s\n' "$*"; }
bad() { printf '  \033[31mFAIL\033[0m  %s\n' "$*"; }
note() { printf '        %s\n' "$*"; }

check_bws() {
  if ! command -v bws >/dev/null 2>&1; then
    bad 'bws: `bws` not on PATH'
    return 1
  fi
  if [ -z "${BWS_ACCESS_TOKEN:-}" ]; then
    bad "bws: BWS_ACCESS_TOKEN not set"
    note "expected /run/secrets/agent-auth/bws-access-token (sops-nix, after rebuild)"
    note "or $CFG/bws/access-token (0600)"
    return 1
  fi
  local out n
  if out="$(timeout 20 bws project list --output json 2>&1)"; then
    n="$(printf '%s' "$out" | grep -c '"id"')" || n=0
    ok "bws: logged in ($n project(s) visible)"
    return 0
  fi
  bad "bws: bws project list failed"
  note "$(printf '%s' "$out" | head -n 3)"
  return 1
}

check_op() {
  if ! command -v op >/dev/null 2>&1; then
    bad '1Password: `op` not on PATH (programs._1password not active yet?)'
    return 1
  fi
  if [ -z "${OP_SERVICE_ACCOUNT_TOKEN:-}" ]; then
    bad "1Password: no service account token"
    note "put it in $OP_TOKEN_FILE (0600) or sops secret agent-auth/op-service-account-token"
    note "or run: agent-auth login"
    return 1
  fi
  local out
  if out="$(timeout 20 op whoami 2>&1)"; then
    ok "1Password: $(printf '%s' "$out" | grep -m1 -iE '^(user|service account|url|account)' || echo 'service account')"
    return 0
  fi
  bad "1Password: op whoami failed"
  note "$(printf '%s' "$out" | head -n 3)"
  return 1
}

check_jules() {
  if ! command -v jules >/dev/null 2>&1; then
    bad 'Jules: `jules` not on PATH'
    return 1
  fi
  local out
  out="$(timeout 30 jules remote list --repo 2>&1)"
  local rc=$?
  if [ $rc -eq 0 ] && ! printf '%s' "$out" | grep -qiE 'login|valid client|error'; then
    ok "Jules: CLI logged in (keyring via ${DBUS_SESSION_BUS_ADDRESS:-no DBus})"
    return 0
  fi
  bad "Jules: CLI not logged in"
  note "$(printf '%s' "$out" | head -n 2)"
  note "run: agent-auth login   (one-time Google OAuth, persists in gnome-keyring)"
  return 1
}

cmd_status() {
  echo "agent-auth status (hook: $HOOK)"
  local rc=0
  check_bws || rc=1
  check_op || rc=1
  check_jules || rc=1
  if [ -n "${JULES_API_KEY:-}" ]; then ok "JULES_API_KEY exported (REST API / jules-mcp)"; else bad "JULES_API_KEY not set"; fi
  if [ -n "${META_API_KEY:-}" ]; then ok "META_API_KEY / MUSE_API_KEY exported (Muse Code & AI SDK)"; else note "META_API_KEY not set"; fi
  return $rc
}

cmd_login() {
  if ! check_op 2>/dev/null; then
    if [ -z "${OP_SERVICE_ACCOUNT_TOKEN:-}" ] && [ -t 0 ]; then
      echo "Paste the 1Password service account token (ops_...), then Enter:"
      local tok
      IFS= read -rs tok
      echo
      if [ -n "$tok" ]; then
        umask 077
        mkdir -p "$(dirname "$OP_TOKEN_FILE")"
        printf '%s\n' "$tok" >"$OP_TOKEN_FILE"
        chmod 600 "$OP_TOKEN_FILE"
        export OP_SERVICE_ACCOUNT_TOKEN="$tok"
        check_op || true
      fi
    fi
  fi
  if ! check_jules >/dev/null 2>&1; then
    if [ -n "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]; then
      jules login
    else
      jules login --no-launch-browser
    fi
    check_jules || true
  fi
  echo
  cmd_status
}

cmd_env() {
  local v
  for v in BASH_ENV BWS_ACCESS_TOKEN OP_SERVICE_ACCOUNT_TOKEN JULES_API_KEY META_API_KEY MUSE_API_KEY CLOUDFLARE_API_TOKEN DBUS_SESSION_BUS_ADDRESS XDG_RUNTIME_DIR; do
    if [ -n "${!v:-}" ]; then echo "$v=set"; else echo "$v=unset"; fi
  done
}

case "${1:-status}" in
status) cmd_status ;;
login) cmd_login ;;
env) cmd_env ;;
-h | --help | help)
  cat <<'EOF'
agent-auth -- check and bootstrap the auth that every hidden shell inherits.

  agent-auth status   OK/FAIL for bws, 1Password (op) and Jules, from a hidden-shell view
  agent-auth login    interactively fix whatever `status` reports as missing
  agent-auth env      which auth variables the hook exports (names only)
EOF
  ;;
*)
  echo "usage: agent-auth [status|login|env]" >&2
  exit 2
  ;;
esac
