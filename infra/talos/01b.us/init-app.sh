#!/usr/bin/env bash

set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

# TODO: accept --argo-app and --argo-app-path and --argo-project and --argo-wait-timeout to overwrite existing env vars

. ./load-env.sh

if [[ -z "$ARGO_APP" ]]; then
  echo "ARGO_APP is empty"
  exit 1
else
  echo "ARGO_APP=$ARGO_APP"
fi

if [[ -z "$ARGO_APP_PATH" ]]; then
  echo "ARGO_APP_PATH is empty"
  exit 1
else
  echo "ARGO_APP_PATH=$ARGO_APP_PATH"
fi

if [[ -z "$ARGO_PROJECT" ]]; then
  echo "ARGO_PROJECT is empty"
  exit 1
else
  echo "ARGO_PROJECT=$ARGO_PROJECT"
fi

if [[ -z "$ARGO_WAIT_TIMEOUT" ]]; then
  echo "ARGO_WAIT_TIMEOUT is empty"
  echo "ARGO_WAIT_TIMEOUT=2m"
  export ARGO_WAIT_TIMEOUT=2m
else
  echo "ARGO_WAIT_TIMEOUT=$ARGO_WAIT_TIMEOUT"
fi

# argocd login 10.10.32.1 --insecure

# argocd-autopilot app create "$ARGO_APP" --app "$GIT_REPO/$ARGO_APP_PATH" -p "$ARGO_PROJECT" --wait-timeout "$ARGO_WAIT_TIMEOUT"
argocd app create "$ARGO_APP" \
  --repo "$GIT_REPO" \
  --path "$ARGO_APP_PATH" \
  --project "$ARGO_PROJECT" \
  --sync-policy auto \
  --dest-server https://kubernetes.default.svc \
  --dest-namespace default
  # --timeout "$ARGO_WAIT_TIMEOUT" \
