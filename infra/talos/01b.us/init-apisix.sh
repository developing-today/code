#!/usr/bin/env bash

set -ex

. ./load-env.sh

if [[ -z "$ARGO_APP" ]]; then
  ARGO_APP="apisix"
else
  echo "ARGO_APP=$ARGO_APP"
fi
if [[ -z "$ARGO_APP_PATH" ]]; then
  ARGO_APP_PATH="app/apisix"
else
  echo "ARGO_APP_PATH=$ARGO_APP_PATH"
fi
if [[ -z "$ARGO_PROJECT" ]]; then
  ARGO_PROJECT="testing"
else
  echo "ARGO_PROJECT=$ARGO_PROJECT"
fi
. ./init-app.sh
