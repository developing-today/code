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
export TALOSCONFIG=talosconfig
talosctl apply-config --insecure --file "./controlplane.yaml" --nodes 10.10.8.188
talosctl apply-config --insecure --file "./controlplane.yaml" --nodes 10.10.15.105
talosctl apply-config --insecure --file "./controlplane.yaml" --nodes 10.10.4.92
talosctl bootstrap --endpoints 10.10.8.188 --nodes 10.10.8.188

# is this right?
talosctl config endpoint 10.10.0.42 10.10.8.188 10.10.15.105 10.10.4.92

talosctl apply-config --insecure --file "./worker.yaml" --nodes 10.10.18.178

talosctl apply-config --insecure --file "./worker.yaml" --nodes 10.10.18.43
talosctl apply-config --insecure --file "./worker.yaml" --nodes 10.10.21.108
talosctl apply-config --insecure --file "./worker.yaml" --nodes 10.10.14.112
talosctl apply-config --insecure --file "./worker.yaml" --nodes 10.10.24.164
talosctl apply-config --insecure --file "./worker.yaml" --nodes 10.10.8.0
talosctl apply-config --insecure --file "./worker.yaml" --nodes 10.10.20.128
talosctl apply-config --insecure --file "./worker.yaml" --nodes 10.10.18.43

# is this right?
talosctl config node 10.10.0.42 10.10.8.188 10.10.15.105 10.10.4.92 10.10.18.178 10.10.18.43 10.10.21.108 10.10.14.112 10.10.24.164 10.10.8.0 10.10.20.128 10.10.18.43
# talosctl config node 10.10.0.42

# talosctl get members
# talosctl get nodestatus
talosctl get volumestatus
talosctl get machinestatus
# talosctl get service
talosctl get memorystats
# talosctl get endpoint
talosctl get diagnostic
talosctl get cpustat
# talosctl get blockdevice
# talosctl get route
talosctl get nodename
talosctl get nodetaintspec
# talosctl get nodeip
# talosctl get modules
# talosctl get hardwareaddress
# talosctl get endpoint
# talosctl get disk
# talosctl get info
# talosctl get identity
talosctl get hostname

talosctl etcd status -n 10.10.8.188,10.10.15.105,10.10.4.92 # workers expected to fail

talosctl kubeconfig ./kubeconfig -n 10.10.0.42
# talosctl kubeconfig -n 10.10.0.42

export KUBECONFIG=kubeconfig

kubectl get nodes
kubectl get nodes -o wide
kubectl get deployments
kubectl get pods

kubectl create deployment hello-world --image=kicbase/echo-server:1.0   --dry-run=client -o yaml | kubectl apply -f -

kubectl get service hello-world
kubectl describe deployment hello-world
kubectl describe service hello-world
kubectl get service hello-world

kubectl expose deployment hello-world --type=LoadBalancer --port=8080

kubectl describe deployment hello-world
kubectl describe service hello-world
kubectl get service hello-world

kubectl get nodes -o wide

kubectl apply -f https://raw.githubusercontent.com/metallb/metallb/v0.13.7/config/manifests/metallb-native.yaml
kubectl -n metallb-system get pods
kubectl apply -f ../kubernetes/infrastructure/networking/metallb/config.yaml

kubectl get nodes
kubectl get nodes -o wide
kubectl get deployments
kubectl get pods
kubectl get nodes -o wide

kubectl describe deployment hello-world
kubectl describe service hello-world
kubectl get service hello-world -o yaml
kubectl get service hello-world

EXTERNAL_IP=$(kubectl get service hello-world -o jsonpath='{.status.loadBalancer.ingress[0].ip}')

echo "EXTERNAL_IP for hello-world is: $EXTERNAL_IP"

PORTS=$(kubectl get service hello-world -o jsonpath='{.spec.ports[*].port}')
echo "PORTS for hello-world are: $PORTS"
for port in $PORTS; do
    echo "Curling $EXTERNAL_IP:$port"
    curl $EXTERNAL_IP:$port
done

cd ..
./save-secrets.sh
