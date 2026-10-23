# shellcheck shell=sh
# agent-shell-env -- auth environment for EVERY shell, including the hidden,
# non-interactive `bash -c` shells that agents, systemd user units, t3code,
# opencode and ssh spawn.
#
# Wiring (see nixos/agent-auth and home/common):
#   * BASH_ENV points here (pam_env + systemd user environment.d), which is the
#     only file a non-interactive, non-login bash ever reads.
#   * ~/.bashrc sources it before the interactive guard, so interactive shells
#     and sshd's remote-command shells get it too.
#
# Rules for this file -- it runs before every `bash -c`, so:
#   * MUST print nothing (would corrupt scp/rsync/agent output parsing).
#   * MUST NOT touch the network or call `op` (startup latency, rate limits).
#   * MUST NOT fail: it may run under an inherited `set -eu` (SHELLOPTS).
#   * Only file PATHS live in the Nix store; secret VALUES are read at runtime.
#   * Values already present in the environment win, so callers can override.
#
# Opt out for a single command with AGENT_SHELL_ENV_DISABLE=1.

if [ -z "${AGENT_SHELL_ENV_DISABLE:-}" ]; then
  # Keep the hook alive in nested shells even when BASH_ENV came from .bashrc.
  export BASH_ENV="@out@/share/agent-shell-env/env.sh"

  __ase_home="${HOME:-}"
  __ase_cfg="${XDG_CONFIG_HOME:-${__ase_home}/.config}"

  # __ase_load VAR FILE... -- export VAR from the first non-empty readable FILE,
  # unless VAR is already set. First line only, no trailing newline, no fork.
  __ase_load() {
    __ase_var="$1"
    shift
    eval "__ase_cur=\${${__ase_var}:-}"
    if [ -z "${__ase_cur}" ]; then
      for __ase_f in "$@"; do
        if [ -r "${__ase_f}" ]; then
          __ase_v=""
          IFS= read -r __ase_v <"${__ase_f}" || :
          if [ -n "${__ase_v}" ]; then
            export "${__ase_var}=${__ase_v}"
            break
          fi
        fi
      done
    fi
    unset __ase_var __ase_cur __ase_f __ase_v
    return 0
  }

  # Secrets come from sops-nix (nixos/agent-auth -> /run/secrets/agent-auth/*,
  # owner `user`, 0400) first, then legacy 0600 files under ~/.config.
  __ase_sec=/run/secrets/agent-auth

  # --- Bitwarden Secrets Manager (`bws`): the access token IS the login ---
  __ase_load BWS_ACCESS_TOKEN \
    "${__ase_sec}/bws-access-token" \
    "${__ase_cfg}/bws/access-token"

  # --- 1Password CLI: service account (headless, no app/biometric prompt) ---
  # `op-personal` strips this to reach personal vaults via the desktop app.
  __ase_load OP_SERVICE_ACCOUNT_TOKEN \
    "${__ase_sec}/op-service-account-token" \
    "${__ase_cfg}/op/service-account-token"

  # --- Jules REST API / @google/jules-mcp ---
  # The `jules` CLI itself ignores this; it uses the OAuth token that
  # `jules login` stored in gnome-keyring (reached over DBus, see below).
  __ase_load JULES_API_KEY \
    "${__ase_sec}/jules-api-key" \
    "${__ase_cfg}/jules/api-key"

  # --- Cloudflare AI gateway (inference-scoped token, not the account one) ---
  __ase_load CLOUDFLARE_ACCOUNT_ID \
    "${__ase_sec}/cloudflare-account-id" \
    "${__ase_cfg}/cloudflare/account-id"
  __ase_load CLOUDFLARE_GATEWAY_ID \
    "${__ase_sec}/cloudflare-gateway-id" \
    "${__ase_cfg}/cloudflare/gateway-id"
  __ase_load CLOUDFLARE_API_TOKEN \
    "${__ase_sec}/cloudflare-ai-inference-token" \
    "${__ase_cfg}/cloudflare/ai-inference-token"

  # --- Meta / Muse Code & AI SDK (META_API_KEY / MUSE_API_KEY) ---
  __ase_load META_API_KEY \
    "${__ase_sec}/muse-sdk-token" \
    "${__ase_cfg}/muse/api-key"
  if [ -n "${META_API_KEY:-}" ] && [ -z "${MUSE_API_KEY:-}" ]; then
    export MUSE_API_KEY="${META_API_KEY}"
  fi

  # --- Session bus, so keyring-backed logins (jules, op app integration) work
  # from units, `env -i` shells and anything else that lost the session env.
  # shellcheck disable=SC3028 # bash sets UID; plain sh falls back to id -u
  __ase_uid="${UID:-}"
  if [ -z "${__ase_uid}" ]; then
    __ase_uid="$(id -u 2>/dev/null)" || __ase_uid=""
  fi
  if [ -n "${__ase_uid}" ] && [ -d "/run/user/${__ase_uid}" ]; then
    if [ -z "${XDG_RUNTIME_DIR:-}" ]; then
      export XDG_RUNTIME_DIR="/run/user/${__ase_uid}"
    fi
    if [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ] && [ -S "/run/user/${__ase_uid}/bus" ]; then
      export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/${__ase_uid}/bus"
    fi
  fi

  unset -f __ase_load
  unset __ase_home __ase_cfg __ase_sec __ase_uid
fi
:
