#!/usr/bin/env bash
if [ -d 01b.us ]; then
  echo "01b.us directory already exists"
  exit 1
fi
echo "*.yaml" >> .gitignore
echo "01b.us*" >> .gitignore
echo "!01b.us.enc" >> .gitignore
echo "!01b.us-*.sh" >> .gitignore
echo "!01b.us_*.sh" >> .gitignore
mkdir -p 01b.us
talosctl gen secrets -o ./01b.us/secrets.yaml
talosctl gen config --with-secrets ./01b.us/secrets.yaml 01b.us https://vip.01b.us:6443 -o 01b.us
./01b.us-save.sh
talosctl apply-config --insecure --file "./01b.us/controlplane.yaml" --nodes 10.10.2.185,10.10.30.13,10.10.4.114
talosctl apply-config --insecure --file "./01b.us/worker.yaml" --nodes 10.10.13.212,10.10.9.224,10.10.18.238,10.10.26.178,10.10.4.187,10.10.14.89,10.10.29.103
talosctl --talosconfig=./01b.us/talosconfig config endpoint 10.10.2.185 10.10.30.13 10.10.4.114
talosctl config merge ./01b.us/talosconfig
talosctl bootstrap --talosconfig ./01b.us/talosconfig --nodes 10.10.2.185 --insecure
# export CONTROL_PLANE_IP=vip.01b.us # use controller nodes directly if etcd is not available
# talosctl config endpoint $CONTROL_PLANE_IP
# talosctl config node $CONTROL_PLANE_IP
talosctl kubeconfig
kubectl get nodes
./01b.us-save.sh
