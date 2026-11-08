#!/usr/bin/env bash
# Install/refresh ready-for-agent (pm-stack Phase 2, dispatch layer)
# on this host.
#
#   ./install.sh            install pinned package + unit, restart on
#                           change (or start when inactive), wait for the
#                           UI, apply config.json (./apply-config.sh,
#                           #1203), print status
#   ./install.sh clone      additionally clone (or fetch) every
#                           repository config.json serves into
#                           ./repos/
#
# Registration note (learned 2026-10-24, first deployment): register
# each clone with the harness through its GraphQL API (loopback
# :6056/graphql, mutation addLocalRepository(path:)). The
# `ready-for-agent add` CLI does its git inspection in-process and
# failed with NOT_A_GIT_REPOSITORY from an interactive SSH shell
# environment on this host; the same inspection succeeds from the
# service's clean environment. A full (non-bare) clone is what the
# first deployment registered and runs with, even though upstream
# documents bare clones as the recommended shape.
#
# The pin: ready-for-agent is an npm package (no container image is
# published). PINNED_VERSION below is the release this stack runs;
# bump it deliberately after reading the upstream release notes —
# never float on @latest.
set -euo pipefail

PINNED_VERSION="1.1.0"

cd "$(dirname "$0")"
DEPLOY_DIR="${PWD}"

# Pending-restart marker (#4839): a package upgrade that never reached
# its restart (e.g. the clone step fails under `set -e` after npm
# replaced the binary) must not be forgotten — the next run would see
# matching on-disk versions and report the pinned version running while
# the old process serves. Written when the package changes, cleared
# only after a verified restart/start.
PENDING_RESTART_FILE="${XDG_STATE_HOME:-${HOME}/.local/state}/ready-for-agent.pending-restart"
restart_pending=false
if [[ -f ${PENDING_RESTART_FILE} ]]; then
  restart_pending=true
fi

# Capture the serving process before changing its executable or unit.
service_was_active=false
previous_main_pid=""
if systemctl --user is-active --quiet ready-for-agent.service; then
  service_was_active=true
  previous_main_pid="$(systemctl --user show ready-for-agent.service \
    --property=MainPID --value 2>/dev/null || true)"
fi
previous_start="$(systemctl --user show ready-for-agent.service \
  --property=ExecMainStartTimestampMonotonic --value 2>/dev/null || true)"

# 1. Pinned npm package into the user prefix (~/.local). The system
#    npm prefix on this host is the read-only Nix store.
current=""
if [[ -x ${HOME}/.local/bin/ready-for-agent ]]; then
  current="$("${HOME}/.local/bin/ready-for-agent" --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || true)"
fi
package_changed=false
if [[ ${current} != "${PINNED_VERSION}" ]]; then
  package_changed=true
  echo "installing ready-for-agent@${PINNED_VERSION} (found: ${current:-none})"
  npm install -g --prefix "${HOME}/.local" "ready-for-agent@${PINNED_VERSION}"
  mkdir -p "$(dirname "${PENDING_RESTART_FILE}")"
  printf '%s\n' "${PINNED_VERSION}" >"${PENDING_RESTART_FILE}"
else
  echo "ready-for-agent@${PINNED_VERSION} already installed"
fi

# 2. Repository clones the harness works from (full clones — the shape
#    the first deployment verified; see the registration note above).
#    Only with the explicit `clone` argument — cloning is a one-time,
#    network-heavy step.
if [[ ${1:-} == clone ]]; then
  mkdir -p "$DEPLOY_DIR/repos"
  # Every repository the declarative config serves (#1203) — not
  # only developing-today/code. Rows are localDir<TAB>cloneUrl.
  while IFS=$'\t' read -r local_dir clone_url; do
    [[ -n $local_dir ]] || continue
    if [[ -d $DEPLOY_DIR/repos/$local_dir ]]; then
      git -C "$DEPLOY_DIR/repos/$local_dir" fetch --all --prune
      echo "fetched $local_dir"
    else
      GIT_TERMINAL_PROMPT=0 git clone "$clone_url" "$DEPLOY_DIR/repos/$local_dir"
      echo "cloned $local_dir"
    fi
  done < <(python3 -c '
import json
with open("config.json") as f:
    cfg = json.load(f)
for repo in cfg["repositories"]:
    print(repo["localDir"] + "\t" + repo["cloneUrl"])
')
fi

# 3. systemd user unit.
installed_version="$("${HOME}/.local/bin/ready-for-agent" --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || true)"
if [[ ${installed_version} != "${PINNED_VERSION}" ]]; then
  echo "ERROR: installed ready-for-agent version is '${installed_version:-unknown}', expected '${PINNED_VERSION}'" >&2
  exit 1
fi

# This upstream service has no version-bearing health endpoint. Only replace
# the unit when its content changed, so an unchanged install is a true
# no-restart path.
source_unit="${DEPLOY_DIR}/systemd/ready-for-agent.service"
installed_unit="${HOME}/.config/systemd/user/ready-for-agent.service"
unit_changed=false
if ! cmp -s "${source_unit}" "${installed_unit}"; then
  unit_changed=true
  install -Dm644 "${source_unit}" "${installed_unit}"
fi

service_changed=false
if [[ ${package_changed} == true || ${unit_changed} == true || ${restart_pending} == true ]]; then
  service_changed=true
fi

if [[ ${unit_changed} == true ]]; then
  systemctl --user daemon-reload
fi
systemctl --user enable ready-for-agent.service
if [[ ${service_was_active} == true ]]; then
  if [[ ${service_changed} == true ]]; then
    systemctl --user restart ready-for-agent.service
    service_action="restarted"
  else
    echo "no package or unit changes detected and no restart pending; leaving ready-for-agent.service running"
    service_action="unchanged"
  fi
else
  systemctl --user start ready-for-agent.service
  service_action="started"
fi

echo "waiting for the harness UI..."
for ((i = 1; i <= 30; i++)); do
  current_start="$(systemctl --user show ready-for-agent.service \
    --property=ExecMainStartTimestampMonotonic --value 2>/dev/null || true)"
  current_main_pid="$(systemctl --user show ready-for-agent.service \
    --property=MainPID --value 2>/dev/null || true)"
  process_verified=false
  if [[ -n ${current_start} && ${current_start} != 0 && -n ${current_main_pid} && ${current_main_pid} != 0 ]]; then
    if [[ ${service_was_active} == true && ${service_changed} == true ]]; then
      # A required restart must replace the serving process, not merely make
      # the unchanged UI endpoint answer successfully.
      if [[ ${current_start} != "${previous_start}" && ${current_main_pid} != "${previous_main_pid}" ]]; then
        process_verified=true
      fi
    elif [[ ${service_was_active} == false ]]; then
      # Starting an inactive unit is itself the process transition. If
      # systemd retained an older start marker, require it to advance too.
      if [[ -z ${previous_start} || ${current_start} != "${previous_start}" ]]; then
        process_verified=true
      fi
    else
      # No changes were detected, so an existing active process is expected.
      process_verified=true
    fi
  fi
  if [[ ${process_verified} == true ]] &&
    systemctl --user is-active --quiet ready-for-agent.service &&
    curl --fail --silent http://127.0.0.1:6056/ >/dev/null; then
    running_version="$("${HOME}/.local/bin/ready-for-agent" --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || true)"
    if [[ ${running_version} != "${PINNED_VERSION}" ]]; then
      echo "ERROR: service restarted, but installed ready-for-agent version is '${running_version:-unknown}', expected '${PINNED_VERSION}'" >&2
      exit 1
    fi
    case "${service_action}" in
    restarted) echo "ready-for-agent@${running_version} is running after restart: http://127.0.0.1:6056" ;;
    started) echo "ready-for-agent@${running_version} is running after start: http://127.0.0.1:6056" ;;
    unchanged) echo "ready-for-agent@${running_version} is already running (no restart): http://127.0.0.1:6056" ;;
    *)
      echo "ERROR: unknown service action '${service_action}'" >&2
      exit 1
      ;;
    esac
    # The new process is verified serving the installed package: any
    # pending restart from an earlier interrupted deployment is now
    # satisfied. The unchanged path never clears — it is only reached
    # with no marker pending, so a marker always forces a restart first.
    if [[ ${service_action} == restarted || ${service_action} == started ]]; then
      rm -f "${PENDING_RESTART_FILE}"
    fi
    # Apply the declarative configuration (#1203) once the new process is
    # verified serving: register the clones and converge the harness
    # settings on config.json, idempotently.
    ./apply-config.sh
    systemctl --user --no-pager status ready-for-agent.service | head -5
    exit 0
  fi
  sleep 5
done
echo "ready-for-agent did not reach the expected process state (${service_action}) and serve the UI; check: journalctl --user -u ready-for-agent" >&2
exit 1
