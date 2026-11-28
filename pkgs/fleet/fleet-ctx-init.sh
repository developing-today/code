#!/usr/bin/env bash
# fleet-ctx-init.sh — capture point 1/5 (#14186): init sourcing contract.
#
# SOURCE this file; do not execute it. Sourcing sets the provisional
# FLEET_CTX_* environment so context persists into everything the shell
# launches afterwards (decision record for #10357, mechanism 1).
#
#   source pkgs/fleet/fleet-ctx-init.sh
#
# Provisional variable set (the exact FLEET_CTX_* list is owned by build
# 2/5, #14188 — this file sets the origin/taxonomy subset 1/5 owns and
# 2/5 extends without renaming):
#
#   FLEET_CTX_ORIGIN     one of: drew-interactive | opencode | claude |
#                        systemd-unit | unknown
#   FLEET_CTX_TASK_ID    queue task id when launched from the fleet queue
#                        (inherited when already set; never invented here)
#   FLEET_CTX_HOST       hostname at capture time
#   FLEET_CTX_ID         join key for stores (task id when known, else
#                        origin+host+pid; build 5/5 joins on it)
#
# Constraints (from #10357, all honored):
# - zero model effort: detection is env/process inspection only.
# - cheap hot path: runs once per session/init; no per-command lookups.
# - no secrets: ids and hashes only. Never export tokens here.
# - absence representable: unknown/interactive shells yield a partial
#   record (origin + host + pid), never an error; exit status is 0
#   unless sourced under `set -e` with an unset variable (guarded).
#
# Idempotent: re-sourcing keeps an already-set FLEET_CTX_TASK_ID and
# refreshes origin/host/id only when they are missing.

# Fail-closed on execution: sourcing is the contract (`init source`,
# not execute). Refuse to run as a subprocess so a bare
# `./fleet-ctx-init.sh` cannot silently drop the exports.
if [[ "${BASH_SOURCE[0]:-}" == "${0}" ]]; then
  echo "fleet-ctx-init: source this file instead of executing it" >&2
  echo "  source ${BASH_SOURCE[0]}" >&2
  return 2 2>/dev/null || exit 2
fi

# --- origin detection (laptop taxonomy, #14186) ---------------------------
# Order matters: systemd wraps the other origins (units run opencode),
# so the unit signal wins and the inner origin is kept as a qualifier.
_fleet_ctx_origin=""
_fleet_ctx_qualifier=""
if [[ -n "${INVOCATION_ID:-}" || -n "${SYSTEMD_EXEC_PID:-}" || -n "${JOURNAL_STREAM:-}" ]]; then
  _fleet_ctx_origin="systemd-unit"
  if [[ -n "${OPENCODE_SESSION_ID:-}" || "${OPENCODE:-}" == "1" || "${AI_AGENT:-}" == "opencode" ]]; then
    _fleet_ctx_qualifier="opencode"
  elif [[ -n "${CLAUDECODE:-}${CLAUDE_CODE:-}${ANTHROPIC_SESSION:-}" ]]; then
    _fleet_ctx_qualifier="claude"
  fi
elif [[ -n "${OPENCODE_SESSION_ID:-}" || "${OPENCODE:-}" == "1" || "${AI_AGENT:-}" == "opencode" || "${MCPX_HARNESS:-}" == "opencode" ]]; then
  _fleet_ctx_origin="opencode"
elif [[ -n "${CLAUDECODE:-}${CLAUDE_CODE:-}${CLAUDE_SESSION_ID:-}${ANTHROPIC_SESSION:-}" ]]; then
  _fleet_ctx_origin="claude"
elif [[ -n "${SSH_TTY:-}${SSH_CLIENT:-}${SSH_CONNECTION:-}" ]]; then
  _fleet_ctx_origin="drew-interactive"
elif [[ $- == *i* ]]; then
  _fleet_ctx_origin="drew-interactive"
else
  _fleet_ctx_origin="unknown"
fi

# --- capture once (never overwrite an inherited record) -------------------
if [[ -z "${FLEET_CTX_ORIGIN:-}" ]]; then
  if [[ -n "$_fleet_ctx_qualifier" ]]; then
    export FLEET_CTX_ORIGIN="${_fleet_ctx_origin}:${_fleet_ctx_qualifier}"
  else
    export FLEET_CTX_ORIGIN="$_fleet_ctx_origin"
  fi
fi
unset _fleet_ctx_origin _fleet_ctx_qualifier

if [[ -z "${FLEET_CTX_HOST:-}" ]]; then
  export FLEET_CTX_HOST="${HOSTNAME:-$(hostname 2>/dev/null || echo unknown)}"
fi

# FLEET_CTX_TASK_ID is inherited when the launcher sets it (queue
# worker, `fleet shell --detach` parent). When absent the record is
# partial by design — never invent one.
if [[ -z "${FLEET_CTX_ID:-}" ]]; then
  if [[ -n "${FLEET_CTX_TASK_ID:-}" ]]; then
    export FLEET_CTX_ID="task-${FLEET_CTX_TASK_ID}"
  else
    export FLEET_CTX_ID="${FLEET_CTX_ORIGIN:-unknown}-${FLEET_CTX_HOST:-unknown}-$$"
  fi
fi

# Systemd unit detail (Q3 default, #10357 §3): when running under a
# unit, record its invocation id alongside — cheap, ids only.
if [[ -n "${INVOCATION_ID:-}" && -z "${FLEET_CTX_INVOCATION:-}" ]]; then
  export FLEET_CTX_INVOCATION="${INVOCATION_ID}"
fi

return 0 2>/dev/null || true
