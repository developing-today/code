#!/usr/bin/env bash

set -e

# TODO: accept --force to overwrite existing env vars

if [[ $- == *x* ]]; then
    ORIGINAL_TRACE=1
else
    ORIGINAL_TRACE=0
fi

function cleanup() {
  if [ "$ORIGINAL_TRACE" -eq 1 ]; then
      set -x
  else
      set +x
  fi
}
trap cleanup EXIT

set +x
# if [[ # GIT_TOKEN is empty
if [[ -z "$GIT_TOKEN" ]]; then
  echo '++ export GIT_TOKEN="$(cat $HOME/auth)" # <redacted>'
  export GIT_TOKEN="$(cat $HOME/auth)"
else
  echo '++ echo "GIT_TOKEN is already set"'
  echo 'GIT_TOKEN is already set'
fi
if [[ -z "$GIT_TOKEN" ]]; then
  echo '++ echo "GIT_TOKEN is empty"'
  echo 'GIT_TOKEN is empty'
  exit 1
fi
set -x
if [[ -z "$GIT_REPO" ]]; then
  export GIT_REPO=https://github.com/developing-today/code
else
  echo "GIT_REPO=$GIT_REPO"
fi
if [[ -z "$KUBECONFIG" ]]; then
  export KUBECONFIG=secrets/kubeconfig
else
  echo "KUBECONFIG=$KUBECONFIG"
fi
if [[ -z "$TALOSCONFIG" ]]; then
  export TALOSCONFIG=secrets/talosconfig
else
  echo "TALOSCONFIG=$TALOSCONFIG"
fi

cleanup
