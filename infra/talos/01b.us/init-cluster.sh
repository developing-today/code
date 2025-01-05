#!/usr/bin/env bash
set -ex
if [ -d secrets ]; then
  echo "secrets directory already exists"
  exit 1
fi
echo "talosconfig*" >>.gitignore
echo "kubeconfig*" >>.gitignore
echo "secrets*" >>.gitignore
echo "!secrets.enc" >>.gitignore
mkdir -p secrets
talosctl gen secrets -o ./secrets/secrets.yaml
talosctl gen config --with-secrets ./secrets/secrets.yaml secrets https://10.10.0.42:6443 -o secrets
./save-secrets.sh
export TALOSCONFIG=secrets/talosconfig
talosctl apply-config --insecure --file "./secrets/controlplane.yaml" --nodes 10.10.8.188 # a1
# talosctl apply-config --insecure --file "./secrets/controlplane.yaml" --nodes 10.10.4.92 # a2 # stopped
# talosctl apply-config --insecure --file "./secrets/controlplane.yaml" --nodes 10.10.15.105 # b1 # stopped
talosctl apply-config --insecure --file "./secrets/controlplane.yaml" --nodes 10.10.12.69 # b1
talosctl apply-config --insecure --file "./secrets/controlplane.yaml" --nodes 10.10.24.137 # c1
talosctl bootstrap --endpoints 10.10.8.188 --nodes 10.10.8.188
# talosctl -n 10.10.24.137 service etcd # c1
# talosctl -n 10.10.8.188 etcd members list
# must remove one to add another
# talosctl etcd remove-member d0ad514b939e5565
# talosctl etcd remove-member fee03360fedfd7c6 # removed b1 10.10.15.105

# is this right?
talosctl config endpoint 10.10.0.42 10.10.8.188 10.10.12.69 10.10.24.137

# talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.18.178
# talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.14.112
# talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.24.164
# talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.8.0
# talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.20.128
# talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.24.136 # b7

talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.18.43
talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.21.108
talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.18.43
talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.31.114 # a2
talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.16.105 # a5
talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.24.60  # c2
talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.4.141  # c3
talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.22.204 # c4
talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.15.10  # c5

talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.25.241 # b2
talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.8.199 # b3
talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.3.175 # b4
talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.27.240 # b5
talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.30.37 # b6
talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.9.213 # b7
talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.31.127 # b8

# is this right?

talosctl config node 10.10.0.42 10.10.8.188 10.10.12.69 10.10.24.137 10.10.18.43 10.10.21.108 10.10.18.43 10.10.31.114 10.10.16.105 10.10.24.60 10.10.4.141 10.10.22.204 10.10.15.10 10.10.25.241 10.10.8.199 10.10.3.175 10.10.27.240 10.10.30.37 10.10.9.213 10.10.31.127

#
# old b?
# 10.10.18.178 \
# 10.10.14.112 \
# 10.10.24.164 \
# 10.10.8.0 \
# 10.10.20.128 \
# 10.10.24.136 \

# talosctl config node 10.10.0.42

# talosctl get members.8.188 10.10.12.69 10.10.4.
# talosctl get nodestatus
talosctl get volumestatus
s
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

talosctl etcd status -n 10.10.8.188,10.10.12.69,10.10.24.137

talosctl kubeconfig ./secrets/kubeconfig -n 10.10.0.42
# talosctl kubeconfig -n 10.10.0.42

export KUBECONFIG=secrets/kubeconfig

kubectl get nodes
kubectl get nodes -o wide
kubectl get deployments
kubectl get pods

kubectl create deployment hello-world --image=kicbase/echo-server:1.0 --dry-run=client -o yaml | kubectl apply -f -

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
kubectl apply -f ../secrets/kubernetes/infrastructure/networking/metallb/config.yaml

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

./save-secrets.sh
