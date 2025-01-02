#!/usr/bin/env bash
set -ex

. ./load-env.sh

if [[ -z "$ARGO_APP" ]]; then
  ARGO_APP="apisix"
else
  echo "ARGO_APP=$ARGO_APP"
fi
if [[ -z "$ARGO_APP_PATH" ]]; then
  ARGO_APP_PATH="manifests/apisix"
else
  echo "ARGO_APP_PATH=$ARGO_APP_PATH"
fi
if [[ -z "$ARGO_PROJECT" ]]; then
  # ARGO_PROJECT="testing"
  ARGO_PROJECT="default"
else
  echo "ARGO_PROJECT=$ARGO_PROJECT"
fi

# helm repo add apisix https://charts.apiseven.com && helm repo update && helm upgrade --install apisix apisix/apisix --create-namespace  --namespace apisix --set dashboard.enabled=true --set ingress-controller.enabled=true --set ingress-controller.config.apisix.serviceNamespace=apisix
. ./init-app.sh
