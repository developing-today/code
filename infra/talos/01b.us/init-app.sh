#!/usr/bin/env bash

set -ex

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

argocd-autopilot app create "$ARGO_APP" --app "$GIT_REPO/$ARGO_APP_PATH" -p "$ARGO_PROJECT" --wait-timeout "$ARGO_WAIT_TIMEOUT"
