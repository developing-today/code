#!/usr/bin/env bash
set -ex
if [ -d secrets ]; then
  echo "secrets directory already exists"
  exit 1
fi
echo "talosconfig*" >> .gitignore
echo "kubeconfig*" >> .gitignore
echo "secrets*" >> .gitignore
echo "!secrets.enc" >> .gitignore
mkdir -p secrets
talosctl gen secrets -o ./secrets/secrets.yaml
talosctl gen config --with-secrets ./secrets/secrets.yaml secrets https://10.10.0.42:6443 -o secrets
./save-secrets.sh
cd secrets
talosctl apply-config --insecure --file "./controlplane.yaml" --nodes 10.10.8.188
talosctl apply-config --insecure --file "./controlplane.yaml" --nodes 10.10.15.105
talosctl apply-config --insecure --file "./controlplane.yaml" --nodes 10.10.4.92
talosctl bootstrap --talosconfig=talosconfig --endpoints 10.10.8.188 --nodes 10.10.8.188

# is this right?
talosctl --talosconfig=talosconfig config endpoint 10.10.0.42 10.10.8.188 10.10.15.105 10.10.4.92

talosctl apply-config --insecure --file "./worker.yaml" --nodes 10.10.18.178

talosctl apply-config --insecure --file "./worker.yaml" --nodes 10.10.18.43
talosctl apply-config --insecure --file "./worker.yaml" --nodes 10.10.21.108
talosctl apply-config --insecure --file "./worker.yaml" --nodes 10.10.14.112
talosctl apply-config --insecure --file "./worker.yaml" --nodes 10.10.24.164
talosctl apply-config --insecure --file "./worker.yaml" --nodes 10.10.8.0
talosctl apply-config --insecure --file "./worker.yaml" --nodes 10.10.20.128
talosctl apply-config --insecure --file "./worker.yaml" --nodes 10.10.18.43

# is this right?
talosctl --talosconfig=talosconfig config node 10.10.0.42 10.10.8.188 10.10.15.105 10.10.4.92 10.10.18.178 10.10.18.43 10.10.21.108 10.10.14.112 10.10.24.164 10.10.8.0 10.10.20.128 10.10.18.43
# talosctl --talosconfig=talosconfig config node 10.10.0.42

# talosctl --talosconfig=talosconfig get members
# talosctl --talosconfig=talosconfig get nodestatus
talosctl --talosconfig=talosconfig get volumestatus
talosctl --talosconfig=talosconfig get machinestatus
# talosctl --talosconfig=talosconfig get service
talosctl --talosconfig=talosconfig get memorystats
# talosctl --talosconfig=talosconfig get endpoint
talosctl --talosconfig=talosconfig get diagnostic
talosctl --talosconfig=talosconfig get cpustat
# talosctl --talosconfig=talosconfig get blockdevice
# talosctl --talosconfig=talosconfig get route
talosctl --talosconfig=talosconfig get nodename
talosctl --talosconfig=talosconfig get nodetaintspec
# talosctl --talosconfig=talosconfig get nodeip
# talosctl --talosconfig=talosconfig get modules
# talosctl --talosconfig=talosconfig get hardwareaddress
# talosctl --talosconfig=talosconfig get endpoint
# talosctl --talosconfig=talosconfig get disk
# talosctl --talosconfig=talosconfig get info
# talosctl --talosconfig=talosconfig get identity
talosctl --talosconfig=talosconfig get hostname

talosctl --talosconfig=talosconfig etcd status -n 10.10.8.188,10.10.15.105,10.10.4.92 # workers expected to fail

talosctl --talosconfig=talosconfig kubeconfig ./kubeconfig -n 10.10.0.42
# talosctl --talosconfig=talosconfig kubeconfig -n 10.10.0.42

kubectl --kubeconfig=kubeconfig get nodes
kubectl --kubeconfig=kubeconfig get nodes -o wide
kubectl --kubeconfig=kubeconfig get deployments
kubectl --kubeconfig=kubeconfig get pods

kubectl --kubeconfig=kubeconfig create deployment hello-world --image=kicbase/echo-server:1.0   --dry-run=client -o yaml | kubectl --kubeconfig=kubeconfig apply -f -

kubectl --kubeconfig=kubeconfig get service hello-world
kubectl --kubeconfig=kubeconfig describe deployment hello-world
kubectl --kubeconfig=kubeconfig describe service hello-world
kubectl --kubeconfig=kubeconfig get service hello-world

kubectl --kubeconfig=kubeconfig expose deployment hello-world --type=LoadBalancer --port=8080

kubectl --kubeconfig=kubeconfig describe deployment hello-world
kubectl --kubeconfig=kubeconfig describe service hello-world
kubectl --kubeconfig=kubeconfig get service hello-world

kubectl --kubeconfig=kubeconfig get nodes -o wide

kubectl --kubeconfig=kubeconfig apply -f https://raw.githubusercontent.com/metallb/metallb/v0.13.7/config/manifests/metallb-native.yaml
kubectl --kubeconfig=kubeconfig -n metallb-system get pods
kubectl --kubeconfig=kubeconfig apply -f ../kubernetes/infrastructure/networking/metallb/config.yaml

kubectl --kubeconfig=kubeconfig get nodes
kubectl --kubeconfig=kubeconfig get nodes -o wide
kubectl --kubeconfig=kubeconfig get deployments
kubectl --kubeconfig=kubeconfig get pods
kubectl --kubeconfig=kubeconfig get nodes -o wide

kubectl --kubeconfig=kubeconfig describe deployment hello-world
kubectl --kubeconfig=kubeconfig describe service hello-world
kubectl --kubeconfig=kubeconfig get service hello-world -o yaml
kubectl --kubeconfig=kubeconfig get service hello-world

EXTERNAL_IP=$(kubectl --kubeconfig=kubeconfig get service hello-world -o jsonpath='{.status.loadBalancer.ingress[0].ip}')

echo "EXTERNAL_IP for hello-world is: $EXTERNAL_IP"

PORTS=$(kubectl --kubeconfig=kubeconfig get service hello-world -o jsonpath='{.spec.ports[*].port}')
echo "PORTS for hello-world are: $PORTS"
for port in $PORTS; do
    echo "Curling $EXTERNAL_IP:$port"
    curl $EXTERNAL_IP:$port
done

cd ..
./save-secrets.sh
